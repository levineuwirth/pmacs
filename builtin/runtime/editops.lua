-- editops.lua --- the editing-conveniences pack.
--
-- goto-line, case ops, transpose, zap-to-char (a kill-chain member;
-- killring owns the ring surface), line move/duplicate/join, region
-- sort/reverse/dedupe, and delete-trailing-whitespace with an opt-in
-- trim-on-save hook. Framing: docs/editing-conveniences-framing.md
-- (Q#EC1..Q#EC10).
--
-- House disciplines this file leans on:
--   * Q#EC2: every text change is ONE pcall'd mutator (one undo
--     step), snapshot-guarded (intercepts may switch window/buffer),
--     exact-effective-triple checked, with right-gravity cursor
--     translation on transformed edits and an UNCONDITIONAL
--     selection clear after any landed edit (a dormant zero-length
--     anchor re-activates on cursor motion).
--   * Locale hygiene: explicit byte ranges and byte comparators
--     only — string.upper/lower, the default string `<`, and the
--     %l/%u/%w/%s/%d pattern classes are ctype/strcoll-backed on the
--     Lua 5.4 backend and shift under os.setlocale.
--   * Word class: ASCII [A-Za-z0-9_] (the word_at_cursor precedent;
--     narrower than the Unicode motion class — named limitation).

pmacs.editops = pmacs.editops or {}

local ed = pmacs.editor

local CHUNK = 4096

-- ---- byte classes (explicit ranges only) ---------------------------

local function is_word_byte(b)
  return (b >= 48 and b <= 57)   -- 0-9
    or (b >= 65 and b <= 90)     -- A-Z
    or (b >= 97 and b <= 122)    -- a-z
    or b == 95                   -- _
end

local function not_word_byte(b)
  return not is_word_byte(b)
end

local function is_nl_byte(b)
  return b == 10
end

local function is_ws_byte(b)
  return b == 32 or b == 9
end

-- ---- chunked scans (giant-line safety; the kill_line precedent) ----

-- First index >= from (< len) whose byte satisfies pred, or nil.
local function scan_forward(buf, from, len, pred)
  local p = from
  while p < len do
    local stop = math.min(p + CHUNK, len)
    local chunk = buf:slice(p, stop)
    for i = 1, #chunk do
      if pred(chunk:byte(i)) then return p + i - 1 end
    end
    p = stop
  end
  return nil
end

-- First index < from (>= 0) whose byte satisfies pred, scanning
-- toward 0, or nil.
local function scan_back(buf, from, pred)
  local p = from
  while p > 0 do
    local start = math.max(p - CHUNK, 0)
    local chunk = buf:slice(start, p)
    for i = #chunk, 1, -1 do
      if pred(chunk:byte(i)) then return start + i - 1 end
    end
    p = start
  end
  return nil
end

-- First occurrence of `needle` (plain bytes) at index >= from, or
-- nil. Windows overlap by #needle - 1 so a match spanning a chunk
-- boundary is seen whole by the next window.
local function find_forward(buf, from, len, needle)
  local n = #needle
  local p = from
  while p < len do
    local stop = math.min(p + CHUNK, len)
    local wstop = math.min(stop + n - 1, len)
    local chunk = buf:slice(p, wstop)
    local i = chunk:find(needle, 1, true)
    if i then return p + i - 1 end
    p = stop
  end
  return nil
end

-- bol/eol of the line containing `pos` (eol = index of its newline,
-- or len for a final bare line; a cursor sitting ON the newline
-- belongs to the line it terminates).
local function line_bounds(buf, pos, len)
  local bol = (scan_back(buf, pos, is_nl_byte) or -1) + 1
  local eol = scan_forward(buf, pos, len, is_nl_byte) or len
  return bol, eol
end

-- ---- UTF-8 (codepoint-exact commands fail closed) ------------------

local function is_cont_byte(b)
  return b >= 0x80 and b <= 0xBF
end

local function cp_len(lead)
  if lead < 0x80 then return 1 end
  if lead >= 0xC2 and lead <= 0xDF then return 2 end
  if lead >= 0xE0 and lead <= 0xEF then return 3 end
  if lead >= 0xF0 and lead <= 0xF4 then return 4 end
  return nil
end

local function single_codepoint(s)
  if #s == 0 then return false end
  local n = cp_len(s:byte(1))
  if not n or n ~= #s then return false end
  for i = 2, #s do
    if not is_cont_byte(s:byte(i)) then return false end
  end
  return true
end

-- Start of the codepoint ending just before `pos`. Returns nil at
-- BOB; nil, "malformed" when the preceding bytes are not one valid
-- UTF-8 codepoint (fail closed — goto_byte guarantees no boundary
-- alignment).
local function prev_cp_start(buf, pos)
  if pos <= 0 then return nil end
  local q = pos - 1
  local steps = 0
  while q > 0 and steps < 3 do
    local b = buf:slice(q, q + 1):byte(1)
    if not is_cont_byte(b) then break end
    q = q - 1
    steps = steps + 1
  end
  local lead = buf:slice(q, q + 1):byte(1)
  local n = cp_len(lead)
  if not n or q + n ~= pos then return nil, "malformed" end
  return q
end

-- ---- ASCII case maps (Q#EC4; never string.upper/lower) -------------

local function ascii_upper(s)
  return (s:gsub("[a-z]", function(c)
    return string.char(c:byte() - 32)
  end))
end

local function ascii_lower(s)
  return (s:gsub("[A-Z]", function(c)
    return string.char(c:byte() + 32)
  end))
end

-- First word byte upcased when it is a letter; every other letter
-- downcased (single-span semantics, Q#EC4).
local function ascii_capitalize(s)
  local first = nil
  for i = 1, #s do
    if is_word_byte(s:byte(i)) then
      first = i
      break
    end
  end
  local lowered = ascii_lower(s)
  if not first then return lowered end
  local b = lowered:byte(first)
  if b >= 97 and b <= 122 then
    return lowered:sub(1, first - 1)
      .. string.char(b - 32)
      .. lowered:sub(first + 1)
  end
  return lowered
end

-- ---- Q#EC2 shared discipline ---------------------------------------

-- Right-gravity translation of `pos` through an effective edit (the
-- indent.lua formula; estop is the PRE-edit end of the range).
local function translate(pos, estart, estop, einserted)
  if pos < estart then return pos end
  if pos > estop then return pos - (estop - estart) + einserted end
  return estart + einserted
end

-- One guarded replace: snapshot -> mutate -> exact verify -> context
-- guard -> cursor (command target on clean, translate+clamp on
-- transformed) -> unconditional selection clear. Returns "clean" |
-- "rejected" | "transformed" | "context".
local function guarded_replace(name, buf, rstart, rstop, text, clean_target)
  local win0 = pmacs.window.current()
  local cursor0 = ed.cursor()
  local ok, estart, estop, einserted = pcall(function()
    return buf:replace(rstart, rstop, text)
  end)
  if not ok then
    ed.set_status(name .. " rejected by buffer intercept")
    return "rejected"
  end
  if pmacs.window.current() ~= win0 or pmacs.window.buffer() ~= buf then
    ed.set_status(name .. ": context changed during edit")
    return "context"
  end
  if estart == rstart and estop == rstop and einserted == #text then
    ed.goto_byte(clean_target)
    ed.clear_selection()
    return "clean"
  end
  ed.set_status(name .. " altered by buffer intercept")
  ed.goto_byte(translate(cursor0, estart, estop, einserted))
  ed.clear_selection()
  return "transformed"
end

-- ---- goto-line (Q#EC3) ---------------------------------------------

pmacs.command.define {
  name = "cursor.goto-line",
  description = "Go to a line by number (1-based; clamps to the buffer).",
  fn = function()
    local origin_fid = pmacs.frontend.id()
    pmacs.minibuffer.read {
      prompt = "Goto line: ",
      history = "goto-line",
      on_accept = function(input)
        -- Milder origin guard (Q#EC6 tail): a prompt completed by
        -- another frontend must not move THAT frontend's cursor.
        if pmacs.frontend.id() ~= origin_fid then
          ed.set_status("goto-line: prompt origin changed; ignored")
          return
        end
        local cap = tostring(input or ""):match("^[ \t]*([0-9]+)[ \t]*$")
        if not cap then
          ed.set_status("goto-line: enter a line number")
          return
        end
        -- Validate and bound BEFORE any state change: "0" clamps to
        -- line 1 (Emacs); the cap keeps huge decimals inside what the
        -- binding's integer conversion accepts (tonumber overflows to
        -- float; math.min returns the integer cap).
        local n = math.max(1, math.min(tonumber(cap), 0x80000000))
        ed.push_jump()
        ed.move_to_line(n - 1)
      end,
    }
  end,
}

-- ---- case ops (Q#EC4) ----------------------------------------------

local function case_command(name, label, xform)
  pmacs.command.define {
    name = name,
    description = label .. " the region, or the word from the cursor.",
    fn = function()
      local buf = pmacs.window.buffer()
      if not buf then
        ed.set_status(name .. ": no buffer")
        return
      end
      local len = buf:len()
      local region = ed.region()
      local rstart, rstop
      if region and region["end"] > region.start then
        rstart, rstop = region.start, region["end"]
      else
        -- First word byte at-or-after the cursor, through that
        -- word's end (Emacs mid-word remainder semantics).
        local ws = scan_forward(buf, ed.cursor(), len, is_word_byte)
        if not ws then
          ed.set_status(name .. ": no word after the cursor")
          return
        end
        rstart = ws
        rstop = scan_forward(buf, ws + 1, len, not_word_byte) or len
      end
      local text = buf:slice(rstart, rstop)
      local new = xform(text)
      if new == text then
        -- Identity: no edit (no undo step, no CRDT op) — but the
        -- cursor still travels and the anchor rule still applies.
        ed.goto_byte(rstop)
        ed.clear_selection()
        return
      end
      guarded_replace(name, buf, rstart, rstop, new, rstop)
    end,
  }
end

case_command("edit.upcase", "Upcase", ascii_upper)
case_command("edit.downcase", "Downcase", ascii_lower)
case_command("edit.capitalize", "Capitalize", ascii_capitalize)

-- ---- transpose (Q#EC5) ---------------------------------------------

pmacs.command.define {
  name = "edit.transpose-chars",
  description = "Swap the characters around the cursor (C-t).",
  fn = function()
    local buf = pmacs.window.buffer()
    if not buf then
      ed.set_status("transpose-chars: no buffer")
      return
    end
    local len = buf:len()
    local cursor = ed.cursor()
    local at_b = nil
    if cursor < len then
      at_b = buf:slice(cursor, cursor + 1):byte(1)
      if is_cont_byte(at_b) then
        ed.set_status("transpose-chars: cursor is inside a multi-byte character")
        return
      end
    end
    if cursor >= len or at_b == 10 then
      -- EOL/EOF: swap the two codepoints BEFORE the cursor (Emacs
      -- special case); cursor stays put.
      local s2, why2 = prev_cp_start(buf, cursor)
      if not s2 then
        ed.set_status(why2 == "malformed"
          and "transpose-chars: malformed UTF-8 before the cursor"
          or "transpose-chars: not enough characters")
        return
      end
      local s1, why1 = prev_cp_start(buf, s2)
      if not s1 then
        ed.set_status(why1 == "malformed"
          and "transpose-chars: malformed UTF-8 before the cursor"
          or "transpose-chars: not enough characters")
        return
      end
      local cp1 = buf:slice(s1, s2)
      local cp2 = buf:slice(s2, cursor)
      guarded_replace("transpose-chars", buf, s1, cursor, cp2 .. cp1, cursor)
    else
      -- Swap the codepoints before and at the cursor; cursor ends
      -- after both (Emacs drag-forward).
      local s1, why = prev_cp_start(buf, cursor)
      if not s1 then
        ed.set_status(why == "malformed"
          and "transpose-chars: malformed UTF-8 before the cursor"
          or "transpose-chars: not enough characters")
        return
      end
      local n2 = cp_len(at_b)
      if not n2 or cursor + n2 > len then
        ed.set_status("transpose-chars: malformed UTF-8 at the cursor")
        return
      end
      local cp1 = buf:slice(s1, cursor)
      local cp2 = buf:slice(cursor, cursor + n2)
      guarded_replace("transpose-chars", buf, s1, cursor + n2, cp2 .. cp1,
        cursor + n2)
    end
  end,
}

pmacs.command.define {
  name = "edit.transpose-words",
  description = "Swap the words around the cursor (M-t).",
  fn = function()
    local buf = pmacs.window.buffer()
    if not buf then
      ed.set_status("transpose-words: no buffer")
      return
    end
    local len = buf:len()
    local cursor = ed.cursor()
    -- W1 = the word containing the last word byte at-or-before
    -- cursor-1 (covers strictly-inside, at-word-start, and separator
    -- positions — the Emacs 30.2 table); fallback: the first word
    -- at-or-after the cursor (BOB / leading separators).
    local s1, e1
    local j = cursor > 0 and scan_back(buf, cursor, is_word_byte) or nil
    if j then
      s1 = (scan_back(buf, j, not_word_byte) or -1) + 1
      e1 = scan_forward(buf, j + 1, len, not_word_byte) or len
    else
      local ws = scan_forward(buf, cursor, len, is_word_byte)
      if not ws then
        ed.set_status("transpose-words: no words to transpose")
        return
      end
      s1 = ws
      e1 = scan_forward(buf, ws + 1, len, not_word_byte) or len
    end
    -- W2 = the first word strictly after W1. Missing -> status, no
    -- edit, NO cursor motion (Emacs errors and moves point; the
    -- point motion is a wart we don't copy).
    local s2 = scan_forward(buf, e1, len, is_word_byte)
    if not s2 then
      ed.set_status("transpose-words: no following word to transpose")
      return
    end
    local e2 = scan_forward(buf, s2 + 1, len, not_word_byte) or len
    local w1 = buf:slice(s1, e1)
    local sep = buf:slice(e1, s2)
    local w2 = buf:slice(s2, e2)
    -- Cursor: end of the replaced span — after W1 in its NEW position.
    guarded_replace("transpose-words", buf, s1, e2, w2 .. sep .. w1, e2)
  end,
}

-- ---- zap (Q#EC6; ring surface owned by killring) -------------------

local function zap_command(cmd_name, label, prompt, up_to)
  pmacs.command.define {
    name = cmd_name,
    description = label,
    fn = function()
      local origin_fid = pmacs.frontend.id()
      pmacs.killring.arm_kill_prompt()
      pmacs.minibuffer.read {
        prompt = prompt,
        history = "zap-char",
        on_accept = function(input)
          -- Origin guard: the session is global, boundaries are
          -- per-frontend, and pointer input breaks the boundary
          -- without closing the prompt. Both checks or no kill.
          if pmacs.frontend.id() ~= origin_fid
            or ed.this_command() ~= cmd_name then
            pmacs.killring.break_chain(origin_fid)
            ed.set_status("zap: prompt origin changed; aborted")
            return
          end
          input = tostring(input or "")
          if not single_codepoint(input) then
            pmacs.killring.break_chain(origin_fid)
            ed.set_status("zap: type a single character")
            return
          end
          local buf = pmacs.window.buffer()
          if not buf then
            pmacs.killring.break_chain(origin_fid)
            ed.set_status("zap: no buffer")
            return
          end
          local len = buf:len()
          local cursor = ed.cursor()
          local p = find_forward(buf, cursor, len, input)
          if not p then
            pmacs.killring.break_chain(origin_fid)
            ed.set_status("zap: no '" .. input .. "' after the cursor")
            return
          end
          local stop = up_to and p or (p + #input)
          if stop <= cursor then
            pmacs.killring.break_chain(origin_fid)
            ed.set_status("zap: already at '" .. input .. "'")
            return
          end
          -- Q#EC2 snapshot around the killring-owned delete.
          local win0 = pmacs.window.current()
          local cursor0 = cursor
          -- A consumed marker means the armed state is no longer
          -- trustworthy (public Lua touched it mid-prompt): fail
          -- closed, no kill.
          if not pmacs.killring.commit_kill_prompt() then
            pmacs.killring.break_chain(origin_fid)
            ed.set_status("zap: prompt state consumed; aborted")
            return
          end
          local ok, kind, estart, estop, eins =
            pmacs.killring.kill_range(cursor, stop)
          if pmacs.window.current() ~= win0
            or pmacs.window.buffer() ~= buf then
            ed.set_status("zap: context changed during edit")
            return
          end
          if ok then
            -- Clean: the delete starts at the cursor; re-seat
            -- explicitly and apply the unconditional anchor clear.
            ed.goto_byte(cursor0)
            ed.clear_selection()
          elseif kind == "transformed" then
            ed.goto_byte(translate(cursor0, estart, estop, eins))
            ed.clear_selection()
          end
          -- rejected: nothing landed; kill_range reported and broke
          -- the chain; no fix-up.
        end,
        on_cancel = function()
          -- Targeted: whichever frontend cancels, the ORIGIN's chain
          -- must break (its this_command is still the zap).
          pmacs.killring.break_chain(origin_fid)
        end,
      }
    end,
  }
end

zap_command("edit.zap-to-char", "Kill through the next occurrence of a character.",
  "Zap to char: ", false)
zap_command("edit.zap-up-to-char", "Kill up to (excluding) the next occurrence of a character.",
  "Zap up to char: ", true)

-- ---- line ops (Q#EC7; plain byte moves, never indentation) ---------

pmacs.command.define {
  name = "edit.move-line-down",
  description = "Swap the cursor line with the line below.",
  fn = function()
    local buf = pmacs.window.buffer()
    if not buf then
      ed.set_status("move-line-down: no buffer")
      return
    end
    local len = buf:len()
    local cursor = ed.cursor()
    local bol, eol = line_bounds(buf, cursor, len)
    if eol >= len then
      ed.set_status("move-line-down: already at the last line")
      return
    end
    local nbol = eol + 1
    local neol = scan_forward(buf, nbol, len, is_nl_byte) or len
    local cur = buf:slice(bol, eol)
    local nxt = buf:slice(nbol, neol)
    local col = math.min(cursor - bol, #cur)
    guarded_replace("move-line-down", buf, bol, neol, nxt .. "\n" .. cur,
      bol + #nxt + 1 + col)
  end,
}

pmacs.command.define {
  name = "edit.move-line-up",
  description = "Swap the cursor line with the line above.",
  fn = function()
    local buf = pmacs.window.buffer()
    if not buf then
      ed.set_status("move-line-up: no buffer")
      return
    end
    local len = buf:len()
    local cursor = ed.cursor()
    local bol, eol = line_bounds(buf, cursor, len)
    if bol == 0 then
      ed.set_status("move-line-up: already at the first line")
      return
    end
    local peol = bol - 1
    local pbol = (scan_back(buf, peol, is_nl_byte) or -1) + 1
    local cur = buf:slice(bol, eol)
    local prev = buf:slice(pbol, peol)
    local col = math.min(cursor - bol, #cur)
    guarded_replace("move-line-up", buf, pbol, eol, cur .. "\n" .. prev,
      pbol + col)
  end,
}

pmacs.command.define {
  name = "edit.duplicate-line",
  description = "Insert a copy of the cursor line below it.",
  fn = function()
    local buf = pmacs.window.buffer()
    if not buf then
      ed.set_status("duplicate-line: no buffer")
      return
    end
    local len = buf:len()
    local cursor = ed.cursor()
    local bol, eol = line_bounds(buf, cursor, len)
    local line = buf:slice(bol, eol)
    local col = math.min(cursor - bol, #line)
    -- Insert "\n"..line AT the line's end: works uniformly for a
    -- middle line and a final line without a trailing newline.
    guarded_replace("duplicate-line", buf, eol, eol, "\n" .. line,
      eol + 1 + col)
  end,
}

pmacs.command.define {
  name = "edit.join-line",
  description = "Join the cursor line onto the previous line (M-^).",
  fn = function()
    local buf = pmacs.window.buffer()
    if not buf then
      ed.set_status("join-line: no buffer")
      return
    end
    local len = buf:len()
    local cursor = ed.cursor()
    local bol, eol = line_bounds(buf, cursor, len)
    if bol == 0 then
      ed.set_status("join-line: already at the first line")
      return
    end
    local peol = bol - 1
    local pbol = (scan_back(buf, peol, is_nl_byte) or -1) + 1
    -- Junction: prev line's trailing whitespace + the newline + the
    -- current line's leading whitespace, replaced by one space — or
    -- nothing when either side of the junction is empty.
    local tws = peol
    while tws > pbol do
      local b = buf:slice(tws - 1, tws):byte(1)
      if is_ws_byte(b) then tws = tws - 1 else break end
    end
    local lwe = bol
    while lwe < eol do
      local b = buf:slice(lwe, lwe + 1):byte(1)
      if is_ws_byte(b) then lwe = lwe + 1 else break end
    end
    local prev_empty = tws == pbol
    local cur_empty = lwe == eol
    local sep = (prev_empty or cur_empty) and "" or " "
    guarded_replace("join-line", buf, tws, lwe, sep, tws)
  end,
}

-- ---- region line ops (Q#EC8) ---------------------------------------

-- Byte-wise line order: never the default string `<` (strcoll).
local function byte_lt(a, b)
  local la, lb = #a, #b
  local n = la < lb and la or lb
  for i = 1, n do
    local ba, bb = a:byte(i), b:byte(i)
    if ba ~= bb then return ba < bb end
  end
  return la < lb
end

local function region_lines_command(name, label, transform)
  pmacs.command.define {
    name = name,
    description = label,
    fn = function()
      local buf = pmacs.window.buffer()
      if not buf then
        ed.set_status(name .. ": no buffer")
        return
      end
      local region = ed.region()
      if not region or region["end"] <= region.start then
        ed.set_status(name .. ": no active region (select the lines first)")
        return
      end
      local len = buf:len()
      -- Whole-line expansion: BOL of the start line through EOL of
      -- the line containing region.end - 1 (a region ending exactly
      -- at a BOL excludes that line), newline included when present.
      local lstart = (scan_back(buf, region.start, is_nl_byte) or -1) + 1
      local eol = scan_forward(buf, region["end"] - 1, len, is_nl_byte)
      local lend = eol and (eol + 1) or len
      local text = buf:slice(lstart, lend)
      local had_nl = text:sub(-1) == "\n"
      local body = had_nl and text:sub(1, -2) or text
      local lines = {}
      local pos = 1
      while true do
        local nl = body:find("\n", pos, true)
        if nl then
          lines[#lines + 1] = body:sub(pos, nl - 1)
          pos = nl + 1
        else
          lines[#lines + 1] = body:sub(pos)
          break
        end
      end
      local newlines, info = transform(lines)
      if info then ed.set_status(info) end
      local newbody = table.concat(newlines, "\n") .. (had_nl and "\n" or "")
      if newbody == text then
        -- Identity: no edit, no undo step; anchor rule still applies.
        ed.goto_byte(lstart)
        ed.clear_selection()
        return
      end
      guarded_replace(name, buf, lstart, lend, newbody, lstart)
    end,
  }
end

region_lines_command("edit.sort-lines", "Sort the selected lines (byte order).",
  function(lines)
    local out = {}
    for i, l in ipairs(lines) do out[i] = l end
    table.sort(out, byte_lt)
    return out
  end)

region_lines_command("edit.reverse-lines", "Reverse the order of the selected lines.",
  function(lines)
    local out = {}
    for i = #lines, 1, -1 do out[#out + 1] = lines[i] end
    return out
  end)

region_lines_command("edit.delete-duplicate-lines",
  "Delete duplicate lines in the region, keeping first occurrences.",
  function(lines)
    local seen, out = {}, {}
    for _, l in ipairs(lines) do
      if not seen[l] then
        seen[l] = true
        out[#out + 1] = l
      end
    end
    return out, string.format("delete-duplicate-lines: %d removed",
      #lines - #out)
  end)

-- ---- delete-trailing-whitespace (Q#EC9) ----------------------------

-- Trailing ' '/'\t' runs, one {start, stop, line} per affected line,
-- ascending. Applied bottom-up so earlier deletes never shift later
-- targets.
local function collect_trailing(buf, len)
  local runs = {}
  local line_no = 1
  local p = 0
  while true do
    local eol = scan_forward(buf, p, len, is_nl_byte) or len
    local s = eol
    while s > p do
      local b = buf:slice(s - 1, s):byte(1)
      if is_ws_byte(b) then s = s - 1 else break end
    end
    if s < eol then
      runs[#runs + 1] = { start = s, stop = eol, line = line_no }
    end
    if eol >= len then break end
    p = eol + 1
    line_no = line_no + 1
  end
  return runs
end

local function trim_active(name)
  local buf = pmacs.window.buffer()
  if not buf then
    ed.set_status(name .. ": no buffer")
    return
  end
  local len = buf:len()
  local runs = collect_trailing(buf, len)
  if #runs == 0 then
    ed.set_status(name .. ": nothing to trim")
    return
  end
  local win0 = pmacs.window.current()
  local cursor0 = ed.cursor()
  local applied = {}
  local failed = nil
  local context_tripped = false
  for i = #runs, 1, -1 do
    local r = runs[i]
    local ok, estart, estop, eins = pcall(function()
      return buf:delete(r.start, r.stop)
    end)
    if not ok then
      -- Rejected: nothing landed for this line; stop the sweep.
      failed = { line = r.line, what = "rejected" }
      break
    end
    applied[#applied + 1] = { estart, estop, eins }
    -- Context guard after EVERY delete: a clean delete's intercept
    -- can switch window/buffer, and the sweep must not keep deleting
    -- through the saved handle behind the new context's back.
    if pmacs.window.current() ~= win0 or pmacs.window.buffer() ~= buf then
      context_tripped = true
      break
    end
    if estart ~= r.start or estop ~= r.stop or eins ~= 0 then
      -- Transformed: the intercept's edit stands (it is in `applied`
      -- for translation); stop the sweep.
      failed = { line = r.line, what = "altered" }
      break
    end
  end
  if context_tripped then
    ed.set_status(name .. ": context changed during edit")
    return
  end
  if failed then
    ed.set_status(string.format("%s: line %d %s by buffer intercept",
      name, failed.line, failed.what))
  else
    ed.set_status(string.format("%s: trimmed %d line%s", name, #applied,
      #applied == 1 and "" or "s"))
  end
  if #applied > 0 then
    -- Translate through every landed effective edit, in application
    -- order; goto_byte clamps. Unconditional anchor clear (step 7).
    local c = cursor0
    for _, t in ipairs(applied) do
      c = translate(c, t[1], t[2], t[3])
    end
    ed.goto_byte(c)
    ed.clear_selection()
  end
end

pmacs.command.define {
  name = "edit.delete-trailing-whitespace",
  description = "Delete trailing spaces and tabs from every line.",
  fn = function()
    trim_active("delete-trailing-whitespace")
  end,
}

-- Opt-in trim-on-save. Getter when nil (the killring.max shape);
-- default OFF — rewriting bytes on save is a policy, not a default.
local trim_enabled = false
function pmacs.editops.trim_on_save(on)
  if on == nil then return trim_enabled end
  trim_enabled = (on ~= false)
  return trim_enabled
end

-- Registered at load time (gated inside) so it runs BEFORE
-- saveplace's cursor-record in the before-save fan-out: editops.lua
-- loads before saveplace.lua (the loader ordering contract, Q#EC9).
pmacs.hook.add("buffer.before-save", function()
  -- Outer pcall: a raised error in a short-circuit hook vetoes the
  -- save; trim failure must report via status, never block a save.
  pcall(function()
    if trim_enabled then
      trim_active("delete-trailing-whitespace (on save)")
    end
  end)
  -- nil return: never a veto.
end)

-- ---- bindings (Q#EC1: all verified free across builtin bind sites) --

local function bind(seq, command)
  pmacs.keymap.bind { scope = "global", sequence = seq, command = command }
end

bind("M-g g", "cursor.goto-line")
bind("M-g M-g", "cursor.goto-line")
bind("M-u", "edit.upcase")
bind("M-l", "edit.downcase")
bind("M-c", "edit.capitalize")
bind("C-t", "edit.transpose-chars")
bind("M-t", "edit.transpose-words")
bind("M-z", "edit.zap-to-char")
bind("M-<up>", "edit.move-line-up")
bind("M-<down>", "edit.move-line-down")
bind("M-^", "edit.join-line")
