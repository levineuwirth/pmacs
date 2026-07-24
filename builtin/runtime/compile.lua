-- compile.lua --- compile-mode / shell-command + unified next-error
-- (Arc 5 stage 1). Framing: docs/compile-mode-framing.md.
--
-- ORDERING CONTRACT: this chunk must load AFTER lsp.lua. It takes
-- over `M-g n` / `M-g p` for the unified error dispatchers, and
-- duplicate bindings are rejected — the takeover is unbind-then-
-- bind, which requires lsp.lua's diag bindings to exist first
-- (Q#CM1/Q#CM5). Its `process.after-tick` subscription is
-- ordering-independent: it pumps only its own proc-id-keyed
-- registry, disjoint from the REPL package's.
--
-- Shape (Q#CM2/Q#CM3): a generated buffer per "slot" (*compilation*,
-- *shell-command*) streams one merged output pipe (`/bin/sh -c
-- "exec 2>&1; <cmdline>"`, TERM=dumb, stdin="null", group=true)
-- through a Lua-side ANSI parser — text appends at a tracked output
-- position, SGR becomes style-overlay spans, CR/BS/erase collapse
-- progress bars. Lines are parsed for error locations exactly once,
-- when their newline lands (Q#CM4). The buffer is read-only via an
-- erroring intercept; module writes bypass it. External edits are
-- survived by a buffer-revision guard with a desync marker and
-- anchor epochs (Q#CM2).

pmacs.compile = pmacs.compile or {}
pmacs.shell = pmacs.shell or {}
pmacs.errors = pmacs.errors or {}

local COMPILATION = "*compilation*"
local SHELL_OUT = "*shell-command*"
local SEARCH_RESULTS = "*search-results*"

local DESYNC_MARKER = "\n[output desynced by external edit]\n"

-- ---------------------------------------------------------------------
-- Unified next-error dispatcher (Q#CM5)
-- ---------------------------------------------------------------------

-- Last claim wins (a deliberate simplification of Emacs's
-- next-error-last-buffer). A compile run claims on spawn; the grep
-- upgrade in commands/default.lua claims on search start. With no
-- claim, the dispatchers fall through to the diagnostic commands —
-- a user who never compiles or greps sees exactly the pre-compile
-- behavior (including diag's wrap).
local claimed_source = nil

function pmacs.errors.claim(source)
  claimed_source = source
end

pmacs.command.define {
  name = "error.next",
  description = "Jump to the next error (compile/grep when claimed; diagnostics otherwise).",
  fn = function()
    if claimed_source then
      claimed_source.next()
    else
      pmacs.command.invoke("diag.next")
    end
  end,
}

pmacs.command.define {
  name = "error.previous",
  description = "Jump to the previous error (compile/grep when claimed; diagnostics otherwise).",
  fn = function()
    if claimed_source then
      claimed_source.previous()
    else
      pmacs.command.invoke("diag.previous")
    end
  end,
}

-- ---------------------------------------------------------------------
-- Error rules (Q#CM4)
-- ---------------------------------------------------------------------

-- Ordered; first match per line wins. Captures follow compiler
-- convention: 1-based line/column (values below 1 fail closed).
-- `severity` is an override; nil falls back to keyword sniffing on
-- the matched line. User-extensible from init.lua.
pmacs.compile.rules = {
  -- rustc/cargo arrow lines: "  --> src/foo.rs:12:4". `[^:]+` (the
  -- framing's spelling), NOT `[^:%s]+` — paths may contain spaces
  -- (PR #113 round-1 finding 3).
  { pattern = "%-%->%s+([^:]+):(%d+):(%d+)", file = 1, line = 2, col = 3 },
  -- gcc/clang/grep-format: "file:line:col:" (also matches most Unix tools)
  { pattern = "([^%s:][^:]*):(%d+):(%d+):", file = 1, line = 2, col = 3 },
  -- Python tracebacks: 'File "foo.py", line 12'
  { pattern = 'File "([^"]+)", line (%d+)', file = 1, line = 2 },
  -- generic two-part: "file:line:"
  { pattern = "([^%s:][^:]*):(%d+):", file = 1, line = 2 },
}

-- A capture index must be a positive, FINITE integer. Fractional
-- indexes read a distinct (absent) table key, not a capture;
-- math.floor(math.huge) == math.huge, so integrality alone does not
-- imply finiteness (round-1 finding 4; round-2 finding 2).
local function is_capture_index(v)
  return type(v) == "number" and v >= 1 and v < math.huge and v == math.floor(v)
end

-- Validate one rule via RAW reads (rawget): a metatable-backed entry
-- whose __index raises must be a skipped malformed entry, not an
-- error thrown through the per-frame pump mid-batch (round-2
-- finding 1). Fail-closed posture: metatable-provided fields are
-- deliberately not honored. Returns a plain-table copy of the
-- validated scalar fields, or nil — the copy is the run's snapshot,
-- immune to post-validation mutation of the user's rule object.
local function validated_rule_copy(rule)
  if type(rule) ~= "table" then return nil end
  local pattern = rawget(rule, "pattern")
  if type(pattern) ~= "string" then return nil end
  -- Probe the pattern against the empty string so a malformed Lua
  -- pattern is caught (and counted in the status note) here at
  -- validation time, not silently at match time.
  if not pcall(string.match, "", pattern) then return nil end
  local file = rawget(rule, "file")
  local line = rawget(rule, "line")
  local col = rawget(rule, "col")
  local severity = rawget(rule, "severity")
  if not is_capture_index(file) then return nil end
  if not is_capture_index(line) then return nil end
  if col ~= nil and not is_capture_index(col) then return nil end
  if severity ~= nil and severity ~= "error" and severity ~= "warning" then
    return nil
  end
  return { pattern = pattern, file = file, line = line, col = col, severity = severity }
end

-- The defaults are a private deep copy taken at load time: an alias
-- of the public table would keep in-place user mutations live after
-- the "using built-in defaults" degradation (round-1 finding 10).
local BUILTIN_RULES = {}
for i, rule in ipairs(pmacs.compile.rules) do
  BUILTIN_RULES[i] = validated_rule_copy(rule)
end

-- Validate the (user-mutable) rule table once per run, fail-closed
-- per entry (Q#CM4): a non-table container degrades to the built-in
-- defaults; malformed entries are skipped; one status note per run
-- counts the skips. Never raises — this feeds the per-frame pump —
-- so the container traversal itself is protected too (a hostile
-- __index on the OUTER table can raise from inside ipairs; round-2
-- finding 1). The returned list holds per-run plain-table copies:
-- validation is a stable, total snapshot, and mutating the user's
-- rule objects after compile.run() cannot alter an in-flight run.
local function validated_rules()
  local rules = pmacs.compile.rules
  if type(rules) ~= "table" then
    pmacs.editor.set_status("compile: pmacs.compile.rules is not a table; using built-in defaults")
    return BUILTIN_RULES, 0
  end
  local valid, skipped = {}, 0
  local ok = pcall(function()
    for _, rule in ipairs(rules) do
      local copy = validated_rule_copy(rule)
      if copy then
        valid[#valid + 1] = copy
      else
        skipped = skipped + 1
      end
    end
  end)
  if not ok then
    pmacs.editor.set_status(
      "compile: pmacs.compile.rules raised during traversal; using built-in defaults")
    return BUILTIN_RULES, 0
  end
  return valid, skipped
end

local function sniff_severity(line)
  local lower = line:lower()
  if lower:find("error", 1, true) then return "error" end
  if lower:find("warning", 1, true) then return "warning" end
  return nil
end

-- ---------------------------------------------------------------------
-- Slots: one streaming generated buffer per name (Q#CM2)
-- ---------------------------------------------------------------------

-- name -> slot. A slot owns its buffer incarnation, overlay handle,
-- streaming state, error list, and the live process (if any).
local slots = {}
-- proc raw id -> { procid, slot, tomb }. Tombstoned entries drop
-- output on arrival but stay registered until their terminal event
-- drains, then forget (Q#CM9 — forget is legal only on terminated
-- processes; removing earlier leaks the supervisor record).
local pump = {}

local function buffer_named(name)
  for _, id in ipairs(pmacs.buffer.list()) do
    local ok, d = pcall(pmacs.describe.buffer, id)
    if ok and d and d.name == name then return id end
  end
  return nil
end

local function slot_for_buffer(buf)
  if not buf then return nil end
  for _, slot in pairs(slots) do
    if slot.buf and slot.buf == buf then return slot end
  end
  return nil
end

--- True when `buf` is one of the module's generated buffers (or the
--- grep panel, which shares the q-target discipline). Used for the
--- never-capture-a-generated-buffer guard (Q#CM11) — also consumed
--- by the project.search upgrade in commands/default.lua.
function pmacs.compile.is_generated_buffer(buf)
  if not buf then return false end
  local ok, d = pcall(pmacs.describe.buffer, buf)
  if not (ok and d) then return false end
  return d.name == COMPILATION or d.name == SHELL_OUT or d.name == SEARCH_RESULTS
end

local UNDO_CHORDS = { "C-/", "C-_", "C-4", "C-x u", "C-?", "C-S-_", "C-x r" }

local function bind_slot_keys(slot)
  local function bind(seq, command)
    pmacs.keymap.bind {
      scope = "buffer", buffer = slot.buf, sequence = seq, command = command,
    }
  end
  bind("RET", "compile.visit-error")
  bind("n", "compile.next-error-line")
  bind("p", "compile.previous-error-line")
  bind("q", "compile.quit")
  bind("C-c C-k", "compile.kill")
  if slot.name == COMPILATION then
    bind("g", "compile.recompile")
  end
  -- All seven shipped undo/redo chords become status no-ops (Q#CM2
  -- layer 1); command/menu undo stays dispatchable and is
  -- guard-recovered by the revision guard (layer 2).
  for _, seq in ipairs(UNDO_CHORDS) do
    bind(seq, "compile.undo-noop")
  end
end

local function slot_buffer_removed(slot)
  -- Killed buffer (Q#CM9): terminate promptly, tombstone the pump
  -- entry (its terminal event still drives forget), drop the handle
  -- so the next run recreates the buffer.
  if slot.proc then
    local entry = pump[slot.proc:raw()]
    if entry then entry.tomb = true end
    pcall(pmacs.process.terminate, slot.proc)
    slot.proc = nil
    pmacs.editor.set_status(slot.label .. ": buffer killed; run terminated")
  end
  slot.buf = nil
  slot.overlay = nil
end

local function ensure_slot(name, label)
  local slot = slots[name]
  if slot and slot.buf and slot.buf:is_valid() then return slot end
  slot = slot or { name = name, label = label }
  slots[name] = slot
  slot.buf = buffer_named(name) or pmacs.buffer.create(name)
  -- Read-only via erroring intercept (the listview idiom); module
  -- writes pass bypass_intercept. Lives as long as the buffer.
  pmacs.buffer.add_intercept(slot.buf, function()
    error(name .. " is read-only")
  end)
  -- Q#P6: semantic frontends round-trip keys here (RET must visit,
  -- not optimistically insert a newline; undo chords must reach the
  -- local no-ops).
  pmacs.buffer.set_round_trip_input(slot.buf, true)
  -- One overlay handle per buffer incarnation, retained; cleared per
  -- run; re-attached after every switch into the buffer (window
  -- overlay attachment is cleared by buffer switches).
  slot.overlay = pmacs.buffer.add_style_overlay(slot.buf)
  pcall(pmacs.buffer.on_removed, slot.buf, function()
    slot_buffer_removed(slot)
  end)
  bind_slot_keys(slot)
  return slot
end

-- ---------------------------------------------------------------------
-- Revision guard + streaming writes (Q#CM2)
-- ---------------------------------------------------------------------

local function count_newlines(s)
  local n = 0
  local i = 0
  while true do
    i = s:find("\n", i + 1, true)
    if not i then return n end
    n = n + 1
  end
end

local function slot_alive(slot)
  return slot.buf ~= nil and slot.buf:is_valid()
end

-- Resync after an external edit (Q#CM2): clamp to the end, reset
-- pending-line state, drop ALL pre-marker in-buffer anchors (a
-- revision carries no edit range; a same-length replace can move
-- newlines with every anchor in bounds), append exactly one
-- newline-delimited marker, and open a fresh anchor epoch — lines
-- completed by subsequent output get trustworthy rows again. The
-- file-location list (M-g n) is preserved across epochs.
local function resync(slot)
  local buf = slot.buf
  for _, e in ipairs(slot.errors) do
    -- Both in-buffer anchors: the display row AND the public byte
    -- anchor — pre-marker byte offsets are exactly as untrustworthy
    -- as rows after an unknown edit (round-1 finding 7).
    e.row = nil
    e.line_start_byte = nil
  end
  local len = buf:len()
  buf:insert(len, DESYNC_MARKER, { bypass_intercept = true })
  slot.out_pos = buf:len()
  -- The marker ends with \n, so the fresh epoch starts a new line.
  slot.line_start = slot.out_pos
  slot.parse_line_start = slot.out_pos
  slot.next_row = count_newlines(buf:slice(0, slot.parse_line_start))
  slot.expected_rev = buf:revision()
end

-- The guard's single checkpoint: nil buffer → false; revision drift
-- → resync (returns true: callers may continue, state is coherent
-- again). Called before every producer write and byte-anchor use,
-- and immediately from the buffer.after-edit subscription.
local function check_rev(slot)
  if not slot_alive(slot) then return false end
  if slot.expected_rev == nil then return true end
  if slot.buf:revision() ~= slot.expected_rev then
    resync(slot)
  end
  return true
end

local function style_is_default(style)
  if not style then return true end
  return style.fg == "default"
    and style.bg == "default"
    and not style.bold
    and not style.italic
    and style.underline == "none"
    and not style.reverse
end

local function add_style_span(slot, from, to)
  if not slot.overlay then return end
  if from >= to then return end
  if style_is_default(slot.cur_style) then return end
  slot.overlay:add(from, to, slot.cur_style)
end

-- True when byte `b` is a UTF-8 continuation byte (0x80–0xBF).
local function is_utf8_continuation(b)
  return b >= 0x80 and b < 0xC0
end

-- Codepoint count of `s` (lead bytes only; `s` always holds complete
-- scalars — parser text events never carry a partial sequence).
local function count_codepoints(s)
  local n = 0
  for i = 1, #s do
    if not is_utf8_continuation(s:byte(i)) then n = n + 1 end
  end
  return n
end

-- Byte length of the first `n` codepoints of `s`, clamped to #s.
local function codepoint_prefix_bytes(s, n)
  local len = #s
  local i = 0
  local seen = 0
  while i < len and seen < n do
    i = i + 1
    while i < len and is_utf8_continuation(s:byte(i + 1)) do
      i = i + 1
    end
    seen = seen + 1
  end
  return i
end

-- Track the current line's start as bytes land (round-5 finding 3):
-- `slot.line_start` is the byte offset where the line containing
-- `out_pos` begins. CR, backspace, and erase-line rewinds read it in
-- O(1); the old per-event scan materialized and walked the ENTIRE
-- preceding buffer (buf:slice(0, pos)) on every CR — quadratic for a
-- progress-heavy command behind megabytes of output. The value
-- advances wherever a \n lands (this helper for appended text; the
-- mid-line newline branch inline) and resets on the recovery paths
-- (run start, resync, raw marker appends). Rewinds never cross it,
-- so it is always ≤ out_pos and always a line start.
local function note_appended(slot, base, text)
  local last = nil
  local search = 1
  while true do
    local idx = text:find("\n", search, true)
    if not idx then break end
    last = idx
    search = idx + 1
  end
  if last then slot.line_start = base + last end
end

-- Append `text` at the tracked output position with overwrite
-- semantics (CR progress bars rewrite the current line in place).
--
-- Overwrites are COLUMN-counted and newline-segmented (PR #113
-- round-4 finding 1; codepoints approximate columns — double-width
-- and combining characters count as one, the framing's documented
-- stance). Each newline-free segment consumes one existing codepoint
-- per incoming codepoint — `abc\ré` yields `ébc`, never the
-- byte-counted `éc` — and LF is NOT an overwrite column: a newline
-- arriving mid-line drops the cursor to a fresh line and the stale
-- remainder of the current line survives in place (terminal
-- semantics), where the byte-counted overwrite wrote `X\n` INTO the
-- line, splitting it and leaving the remainder as a ghost line the
-- parser saw again at EOF.
--
-- UTF-8 safety (round-3 finding 1) is per-edit: every segment holds
-- complete scalars (the parser never splits one, and \n is ASCII)
-- and every consumed range covers whole existing codepoints, so the
-- rope is valid UTF-8 after each step and the byte-native CRDT edit
-- never rejects a range. `out_pos` stays on codepoint boundaries by
-- induction: it moves to end-of-segment, line starts (after \n), or
-- a boundary-aligned backspace target.
local function emit_text(slot, text)
  if #text == 0 then return end
  local buf = slot.buf
  local idx = 1
  while idx <= #text do
    local len = buf:len()
    local pos = math.min(slot.out_pos, len)
    if pos >= len then
      -- Append fast path: nothing ahead to overwrite, so the whole
      -- remainder (newlines included) lands as one edit.
      local rest = text:sub(idx)
      buf:insert(len, rest, { bypass_intercept = true })
      slot.out_pos = len + #rest
      note_appended(slot, len, rest)
      add_style_span(slot, len, len + #rest)
      return
    end
    local nl = text:find("\n", idx, true)
    if nl == idx then
      -- Newline while mid-line: cursor to a fresh line; the stale
      -- remainder stays. The \n is appended past it, never written
      -- over it.
      buf:insert(len, "\n", { bypass_intercept = true })
      slot.out_pos = len + 1
      slot.line_start = len + 1
      idx = idx + 1
    else
      local seg = text:sub(idx, (nl or #text + 1) - 1)
      -- The current line's remainder — the only bytes an overwrite
      -- may touch. No \n exists at or past out_pos: rewinds stay
      -- within the final line and \n is only ever appended.
      local tail = buf:slice(pos, len)
      local ow = codepoint_prefix_bytes(tail, count_codepoints(seg))
      buf:replace(pos, pos + ow, seg, { bypass_intercept = true })
      slot.out_pos = pos + #seg
      add_style_span(slot, pos, pos + #seg)
      idx = idx + #seg
    end
  end
end

-- The current unterminated line runs from the tracked
-- `slot.line_start` (per-slot, updated at every \n — NOT
-- `parse_line_start`, which only advances once per batch: a CR
-- arriving in the same batch as earlier completed lines must rewind
-- to the start of the CURRENT line, not the batch's first line) to
-- buf:len() — no newline ever exists past out_pos (output is
-- append-only except CR/BS rewinds within the current line).
local function apply_events(slot, events)
  local buf = slot.buf
  for _, ev in ipairs(events) do
    local kind = ev.kind
    if kind == "text" then
      emit_text(slot, ev.text)
    elseif kind == "set_style" then
      slot.cur_style = ev.style
    elseif kind == "carriage_return" then
      slot.out_pos = slot.line_start
    elseif kind == "backspace" then
      -- Step back over one whole CODEPOINT, not one byte — a
      -- mid-codepoint out_pos would make the next overwrite split
      -- the character (round-3 finding 1).
      local ls = slot.line_start
      if slot.out_pos > ls then
        local prefix = slot.buf:slice(ls, slot.out_pos)
        local i = #prefix
        while i > 1 and is_utf8_continuation(prefix:byte(i)) do
          i = i - 1
        end
        slot.out_pos = ls + i - 1
      end
    elseif kind == "erase_to_eol" then
      local len = buf:len()
      if slot.out_pos < len then
        buf:delete(slot.out_pos, len, { bypass_intercept = true })
      end
    elseif kind == "erase_line" then
      local ls = slot.line_start
      local len = buf:len()
      if ls < len then
        buf:delete(ls, len, { bypass_intercept = true })
      end
      slot.out_pos = ls
    end
    -- alt-screen suppression happens inside the parser; titles and
    -- shell-integration markers are irrelevant to a compile buffer.
  end
end

-- ---------------------------------------------------------------------
-- Line parsing (Q#CM4)
-- ---------------------------------------------------------------------

local SEVERITY_STYLE = {
  error = { fg = 1 }, -- indexed red
  warning = { fg = 3 }, -- indexed yellow
}

-- A stored coordinate must be a finite integer ≥ 1: `%d+` happily
-- captures digit runs whose tonumber is astronomically large or
-- math.huge, and an unbounded value would drive the cursor walk
-- loops effectively forever (round-1 finding 1). The cursor walk
-- also clamps independently — belt and braces.
local function valid_coordinate(n)
  return n ~= nil and n >= 1 and n < math.huge and n == math.floor(n)
end

local function parse_line(slot, line, abs_start)
  if not slot.parse_errors then return end
  for _, rule in ipairs(slot.rules) do
    -- Capture EVERYTHING the pattern produced: validation accepts
    -- any positive integer index, so truncating at three silently
    -- misread four-capture rules (round-1 finding 4).
    local caps = { pcall(string.match, line, rule.pattern) }
    local ok = table.remove(caps, 1)
    if ok and caps[1] then
      local file = caps[rule.file]
      local lnum = tonumber(caps[rule.line])
      local cnum = rule.col and tonumber(caps[rule.col]) or nil
      -- 1-based contract; below-1, non-integral, and non-finite
      -- captures fail closed. A rule that NAMES a column capture the
      -- match didn't produce also fails closed (silently storing
      -- column 0 would misreport the location).
      local col_ok = (rule.col == nil and cnum == nil) or valid_coordinate(cnum)
      if type(file) == "string" and valid_coordinate(lnum) and col_ok then
        local severity = rule.severity or sniff_severity(line)
        slot.errors[#slot.errors + 1] = {
          file = file,
          line = lnum - 1,
          col = cnum and (cnum - 1) or 0,
          severity = severity,
          line_start_byte = abs_start,
          row = slot.next_row,
        }
        local style = severity and SEVERITY_STYLE[severity]
        if style and slot.overlay then
          slot.overlay:add(abs_start, abs_start + #line, style)
        end
        return
      end
      -- Fail-closed match: fall through to later rules.
    end
  end
end

-- Parse every newly completed line exactly once (Q#CM4). Rows are
-- counted per completed line so RET/n/p can map cursor rows to
-- entries without rescanning the buffer.
local function parse_new_lines(slot)
  local buf = slot.buf
  local len = buf:len()
  if slot.parse_line_start >= len then return end
  local chunk = buf:slice(slot.parse_line_start, len)
  local search = 1
  while true do
    local nl = chunk:find("\n", search, true)
    if not nl then break end
    parse_line(slot, chunk:sub(search, nl - 1), slot.parse_line_start + search - 1)
    slot.next_row = slot.next_row + 1
    search = nl + 1
  end
  slot.parse_line_start = slot.parse_line_start + search - 1
end

-- ---------------------------------------------------------------------
-- Run lifecycle (Q#CM3/Q#CM9/Q#CM11)
-- ---------------------------------------------------------------------

local function project_root_of_active()
  local buf = pmacs.window.buffer()
  if not buf then return nil end
  local ok, path = pcall(function() return buf:path() end)
  if not (ok and path) then return nil end
  local ok2, proj = pcall(pmacs.project.detect, path)
  if ok2 and proj and proj.root then return proj.root end
  return nil
end

-- The daemon's actual working directory: the last-resort cwd when
-- there is no explicit opt and no detectable project. Resolving it
-- (rather than leaving nil and printing "(inherited)") gives the
-- header a real path and relative error files an explicit base
-- (round-1 finding 8).
local function daemon_working_directory()
  local ok, id = pcall(pmacs.instance.identity)
  if ok and type(id) == "table" and type(id.working_directory) == "string" then
    return id.working_directory
  end
  return nil
end

local function format_exit_marker(label, ev)
  if ev.kind == "exited" then
    return string.format("\n[%s exited with code %d]\n", label, ev.code or 0)
  elseif ev.kind == "signaled" then
    return string.format("\n[%s killed by %s]\n", label, ev.signal or "signal")
  elseif ev.kind == "crashed" then
    return string.format("\n[%s crashed: %s]\n", label, ev.error or "unknown")
  end
  return string.format("\n[%s exited]\n", label)
end

-- Plain append at end, no overwrite/style tracking — markers and
-- headers. LOCAL by design: a global here would let user config
-- shadow a helper the terminal-event path depends on, and an error
-- thrown from that shadow would consume the terminal event before
-- pump cleanup/forget ran (round-1 finding 5).
local function emit_text_raw(slot, text)
  local buf = slot.buf
  local base = buf:len()
  buf:insert(base, text, { bypass_intercept = true })
  slot.out_pos = buf:len()
  note_appended(slot, base, text)
  if slot.parse_line_start > slot.out_pos then
    slot.parse_line_start = slot.out_pos
  end
end

-- Terminal event: drain the parser's cross-feed state (an
-- incomplete UTF-8 sequence at process EOF can never complete — the
-- parser's finish() emits its replacement character, round-1
-- finding 9), finalize the pending unterminated line (a final
-- diagnostic emitted without a trailing newline is complete at EOF
-- and must not be dropped — Q#CM4), then the exit marker.
local function finish_run(slot, ev)
  if not check_rev(slot) then return end
  local buf = slot.buf
  if slot.parser then
    apply_events(slot, slot.parser:finish())
  end
  local len = buf:len()
  if slot.parse_errors and slot.parse_line_start < len then
    parse_line(slot, buf:slice(slot.parse_line_start, len), slot.parse_line_start)
    slot.next_row = slot.next_row + 1
    slot.parse_line_start = len
  end
  slot.out_pos = buf:len()
  emit_text_raw(slot, format_exit_marker(slot.label, ev))
  slot.expected_rev = buf:revision()
  if ev.kind == "exited" and (ev.code or 0) == 0 then
    pmacs.editor.set_status(slot.label .. ": finished")
  elseif ev.kind == "exited" then
    pmacs.editor.set_status(string.format("%s: exited abnormally with code %d", slot.label, ev.code))
  else
    pmacs.editor.set_status(slot.label .. ": " .. ev.kind)
  end
end

local function feed_bytes(slot, bytes)
  if not check_rev(slot) then return end
  apply_events(slot, slot.parser:feed(bytes))
  parse_new_lines(slot)
  slot.expected_rev = slot.buf:revision()
end

pmacs.hook.add("process.after-tick", function()
  for raw, entry in pairs(pump) do
    local events = pmacs.process.events_take(entry.procid)
    for _, ev in ipairs(events) do
      local kind = ev.kind
      if kind == "stdout" or kind == "stderr" then
        -- stderr cannot arrive (fd2 = fd1 at the child boundary),
        -- but if it somehow does, route it through the same parser
        -- rather than dropping user output (the REPL's posture).
        if not entry.tomb and slot_alive(entry.slot) then
          feed_bytes(entry.slot, ev.bytes)
        end
      elseif kind == "exited" or kind == "signaled" or kind == "crashed" then
        if not entry.tomb and slot_alive(entry.slot) then
          finish_run(entry.slot, ev)
        end
        if entry.slot.proc and entry.slot.proc:raw() == raw then
          entry.slot.proc = nil
        end
        pump[raw] = nil
        pcall(pmacs.process.forget, entry.procid)
      end
    end
  end
end)

-- Immediate command-path recovery (Q#CM2 trigger a): M-x/menu edits
-- fire buffer.after-edit; hook edits don't re-fire the hook, so the
-- resync marker can be appended from here safely. Covers the
-- undo-after-completed-run case where no pump event will ever come.
pmacs.hook.add("buffer.after-edit", function()
  local slot = slot_for_buffer(pmacs.window.buffer())
  if slot then check_rev(slot) end
end)

-- Overlay re-attach on ANY switch path landing on a slot buffer
-- (window overlay attachment is cleared by buffer switches; the
-- jump_back binding now fires this hook too, so RET → M-, keeps its
-- styling).
pmacs.hook.add("buffer.after-switch", function()
  local slot = slot_for_buffer(pmacs.window.buffer())
  if slot and slot.overlay and slot_alive(slot) then
    pcall(pmacs.buffer.attach_style_overlay, slot.buf, slot.overlay)
  end
end)

-- Start a run in `slot`. Shared by compile and shell-command; grep
-- has its own worker path.
local function start_run(slot, cmdline, opts)
  opts = opts or {}
  -- Bottom-panel arc (Q#BP11b): validate placement BEFORE the run
  -- supersedes anything, rewrites the buffer, or spawns a process, so
  -- an unknown value leaves no half-started run behind. In Stages 1-2
  -- omission means "current"; Stage 3 flips the default.
  local display = opts.display
  if display ~= nil and display ~= "current" and display ~= "panel" then
    error(string.format(
      "compile.run: unknown display %q (expected \"current\" or \"panel\")",
      tostring(display)))
  end
  -- q-target discipline (Q#CM11): capture only when coming from a
  -- non-generated buffer, so `g` reruns don't re-capture and
  -- compile → g → q restores the original buffer.
  local cur = pmacs.window.buffer()
  if cur and not pmacs.compile.is_generated_buffer(cur) then
    slot.prev = cur
  end
  local cwd = opts.cwd or project_root_of_active() or daemon_working_directory()

  -- Supersede (Q#CM9): terminate the old group and tombstone its
  -- pump entry; its terminal event still drives forget.
  if slot.proc then
    local entry = pump[slot.proc:raw()]
    if entry then entry.tomb = true end
    pcall(pmacs.process.terminate, slot.proc)
    slot.proc = nil
    pmacs.editor.set_status(slot.label .. ": superseded previous run")
  end

  -- Fresh run state. Only error-parsing slots touch the rule table
  -- at all: shell-command performs no parsing, so it must neither
  -- surface compile-rule warnings nor fail on a hostile rule
  -- container (round-2 finding 3).
  if slot.parse then
    slot.rules, slot.skipped_rules = validated_rules()
  else
    slot.rules, slot.skipped_rules = {}, 0
  end
  slot.parse_errors = slot.parse
  slot.errors = {}
  slot.err_index = 0
  slot.cur_style = nil
  slot.parser = pmacs.ansi.parser()
  slot.overlay:clear()
  local buf = slot.buf
  local len = buf:len()
  if len > 0 then buf:delete(0, len, { bypass_intercept = true }) end
  -- The identity fallback should always resolve; "(unknown)" only
  -- survives if the instance API itself failed.
  local header = string.format("$ %s\nDirectory: %s\n\n", cmdline, cwd or "(unknown)")
  buf:insert(0, header, { bypass_intercept = true })
  slot.out_pos = buf:len()
  -- The header ends with \n, so output starts on a fresh line.
  slot.line_start = slot.out_pos
  slot.parse_line_start = slot.out_pos
  slot.next_row = count_newlines(header)
  slot.expected_rev = buf:revision()
  slot.cwd = cwd
  if slot.parse and slot.skipped_rules > 0 then
    pmacs.editor.set_status(
      string.format("compile: skipped %d malformed rule entr%s",
        slot.skipped_rules, slot.skipped_rules == 1 and "y" or "ies"))
  end

  -- Spawn (Q#CM3): pipes, merged stderr at the child boundary, null
  -- stdin, own process group, TERM=dumb.
  local spec = {
    label = slot.label,
    command = "/bin/sh",
    args = { "-c", "exec 2>&1; " .. cmdline },
    env = { TERM = "dumb" },
    stdin = "null",
    group = true,
  }
  if cwd then spec.cwd = cwd end
  local ok, proc = pcall(pmacs.process.spawn, spec)
  -- switch_buffer synchronously fires buffer.after-switch, whose
  -- subscription above attaches the overlay — a second explicit
  -- attach here stacked a duplicate render view per run (round-5
  -- finding 1; translation itself is buffer-level and unaffected by
  -- attachment count).
  -- The FIRST display of this run is the side-affine one (Q#BP3): a
  -- persistent *compilation* already visible in a document window must
  -- not preempt the requested panel. Compile output is passive, so it
  -- takes `select = false` explicitly; a recompile simply reuses the
  -- panel it is already in.
  if display == "panel" then
    pmacs.window.display(slot.buf, { side = "bottom", select = false })
  else
    pmacs.window.switch_buffer(slot.buf)
  end
  if not ok then
    emit_text_raw(slot, string.format("[%s spawn failed: %s]\n", slot.label, tostring(proc)))
    slot.expected_rev = buf:revision()
    pmacs.editor.set_status(slot.label .. ": spawn failed")
    return nil
  end
  slot.proc = proc
  pump[proc:raw()] = { procid = proc, slot = slot, tomb = false }
  return proc
end

-- ---------------------------------------------------------------------
-- Navigation (Q#CM5/Q#CM6)
-- ---------------------------------------------------------------------

-- Cursor walk via primitives so overlay observers see the motion
-- (the lsp.lua visit idiom; 0-based line/col; the col walk shares
-- lsp.lua's inherited per-codepoint residual). Both walks stop when
-- movement stops moving — a diagnostic pointing past EOF/EOL clamps
-- there instead of looping to its nominal coordinate (round-1
-- finding 1; parse-time validation bounds the values, the clamp
-- bounds the walk regardless).
local function move_active_cursor_to(line, col)
  pmacs.editor.move_line_start()
  while pmacs.editor.cursor_line() > 0 do
    pmacs.editor.move_up()
  end
  for _ = 1, line do
    local before = pmacs.editor.cursor_line()
    pmacs.editor.move_down()
    if pmacs.editor.cursor_line() == before then break end -- EOF
  end
  local row = pmacs.editor.cursor_line()
  for _ = 1, col do
    local before = pmacs.editor.cursor()
    pmacs.editor.move_right()
    if pmacs.editor.cursor() == before then break end -- buffer end
    if pmacs.editor.cursor_line() ~= row then
      -- Ran off the line's end onto the next row: step back to EOL.
      pmacs.editor.move_left()
      break
    end
  end
end

local function resolve_error_path(slot, file)
  if file:sub(1, 1) == "/" then return file end
  if slot.cwd then return slot.cwd .. "/" .. file end
  -- No explicit cwd: the child inherited the editor's, and so does
  -- find_or_open's relative resolution — pass through unchanged.
  return file
end

-- Visit `slot.errors[idx]` (the visit_location discipline: jump
-- ring, pcall'd open, status on failure). Re-seats the walk index.
local function visit_error(slot, idx)
  local e = slot.errors[idx]
  if not e then return end
  local path = resolve_error_path(slot, e.file)
  pmacs.editor.push_jump()
  -- Bottom-panel arc (Q#BP11b): RET from a compilation PANEL opens the
  -- source in the document target, leaving the panel where it is.
  local ok, err = pcall(pmacs.window.display_file, path, { select = true })
  if not ok then
    pmacs.editor.jump_back()
    pmacs.editor.set_status(slot.label .. ": failed to open " .. path .. ": " .. tostring(err))
    return
  end
  move_active_cursor_to(e.line, e.col)
  slot.err_index = idx
end

local function compile_slot()
  local slot = slots[COMPILATION]
  if slot and slot_alive(slot) then return slot end
  return nil
end

local function claim_compile_source(slot)
  pmacs.errors.claim {
    name = "compile",
    next = function()
      if #slot.errors == 0 then
        pmacs.editor.set_status("compile: no errors parsed")
        return
      end
      if slot.err_index >= #slot.errors then
        pmacs.editor.set_status("no more errors")
        return
      end
      visit_error(slot, slot.err_index + 1)
    end,
    previous = function()
      if slot.err_index <= 1 then
        pmacs.editor.set_status("no more errors")
        return
      end
      visit_error(slot, slot.err_index - 1)
    end,
  }
end

-- Row → anchored entry index for RET (dropped anchors excluded).
local function entry_on_row(slot, row)
  for i, e in ipairs(slot.errors) do
    if e.row == row then return i end
  end
  return nil
end

pmacs.command.define {
  name = "compile.visit-error",
  description = "Visit the error location on the current line of a compile/shell buffer.",
  fn = function()
    local slot = slot_for_buffer(pmacs.window.buffer())
    if not slot then return end
    if not check_rev(slot) then return end
    local idx = entry_on_row(slot, pmacs.editor.cursor_line())
    if not idx then
      pmacs.editor.set_status("no error on this line")
      return
    end
    visit_error(slot, idx)
  end,
}

local function move_to_row(row)
  local cur = pmacs.editor.cursor_line()
  while cur < row do
    pmacs.editor.move_down()
    cur = cur + 1
  end
  while cur > row do
    pmacs.editor.move_up()
    cur = cur - 1
  end
  pmacs.editor.move_line_start()
end

local function nearest_anchored(slot, from_row, direction)
  local best = nil
  for _, e in ipairs(slot.errors) do
    if e.row then
      if direction > 0 and e.row > from_row and (not best or e.row < best) then
        best = e.row
      elseif direction < 0 and e.row < from_row and (not best or e.row > best) then
        best = e.row
      end
    end
  end
  return best
end

pmacs.command.define {
  name = "compile.next-error-line",
  description = "Move to the next error line within the compile buffer (no visit).",
  fn = function()
    local slot = slot_for_buffer(pmacs.window.buffer())
    if not slot then return end
    if not check_rev(slot) then return end
    local row = nearest_anchored(slot, pmacs.editor.cursor_line(), 1)
    if not row then
      pmacs.editor.set_status("no more errors")
      return
    end
    move_to_row(row)
  end,
}

pmacs.command.define {
  name = "compile.previous-error-line",
  description = "Move to the previous error line within the compile buffer (no visit).",
  fn = function()
    local slot = slot_for_buffer(pmacs.window.buffer())
    if not slot then return end
    if not check_rev(slot) then return end
    local row = nearest_anchored(slot, pmacs.editor.cursor_line(), -1)
    if not row then
      pmacs.editor.set_status("no more errors")
      return
    end
    move_to_row(row)
  end,
}

pmacs.command.define {
  name = "compile.quit",
  description = "Leave the compile/shell buffer, restoring the previous buffer.",
  fn = function()
    local slot = slot_for_buffer(pmacs.window.buffer())
    if not slot then return end
    -- Bottom-panel arc (Q#BP11b): in a side window, `q` deletes or
    -- restores the PRESENTATION rather than leaving a source buffer
    -- stranded in the panel slot. Capability fallback and pre-arc
    -- placement keep today's previous-buffer restore below.
    local params = pmacs.window.params()
    if params and params.side and params.quit_action then
      pmacs.window.quit()
      return
    end
    local target = slot.prev
    if not (target and target:is_valid()) then
      target = buffer_named("*scratch*") or pmacs.buffer.create("*scratch*")
    end
    pmacs.window.switch_buffer(target)
  end,
}

pmacs.command.define {
  name = "compile.kill",
  description = "Terminate the running compilation (SIGTERM to its process group).",
  fn = function()
    local slot = slot_for_buffer(pmacs.window.buffer()) or compile_slot()
    if not (slot and slot.proc) then
      pmacs.editor.set_status("compile: no compilation running")
      return
    end
    pcall(pmacs.process.terminate, slot.proc)
    pmacs.editor.set_status(slot.label .. ": killed")
  end,
}

pmacs.command.define {
  name = "compile.undo-noop",
  description = "Undo is disabled in generated compile/shell buffers.",
  fn = function()
    pmacs.editor.set_status("generated buffer: undo disabled")
  end,
}

-- ---------------------------------------------------------------------
-- Entry points (Q#CM11)
-- ---------------------------------------------------------------------

--- Programmatic compile entry. `opts.cwd` overrides the resolved
--- working directory. Stores the recompile state on success.
function pmacs.compile.run(cmdline, opts)
  if type(cmdline) ~= "string" or #cmdline == 0 then
    error("pmacs.compile.run: cmdline must be a non-empty string")
  end
  local slot = ensure_slot(COMPILATION, "compile")
  slot.parse = true
  local proc = start_run(slot, cmdline, opts)
  if proc then
    pmacs.compile._last = { cmdline = cmdline, cwd = slot.cwd }
    claim_compile_source(slot)
  end
  return proc
end

--- The run's parsed error locations, oldest first. Public getter
--- (per API conventions): `{ file, line, col, severity,
--- line_start_byte }` with 0-based line/col.
function pmacs.compile.errors()
  local slot = slots[COMPILATION]
  local out = {}
  if not slot then return out end
  for _, e in ipairs(slot.errors) do
    out[#out + 1] = {
      file = e.file,
      line = e.line,
      col = e.col,
      severity = e.severity,
      line_start_byte = e.line_start_byte,
    }
  end
  return out
end

--- Programmatic shell-command entry (Q#CM8): same machinery, no
--- error parsing, no error-source claim.
function pmacs.shell.command(cmdline, opts)
  if type(cmdline) ~= "string" or #cmdline == 0 then
    error("pmacs.shell.command: cmdline must be a non-empty string")
  end
  local slot = ensure_slot(SHELL_OUT, "shell")
  slot.parse = false
  return start_run(slot, cmdline, opts)
end

pmacs.command.define {
  name = "compile.run",
  description = "Compile: run a command in a streaming *compilation* buffer (M-x compile).",
  fn = function()
    local last = pmacs.compile._last
    pmacs.minibuffer.read {
      prompt = "Compile command: ",
      history = "compile",
      initial = last and last.cmdline or "",
      on_accept = function(cmdline)
        if cmdline == nil or cmdline == "" then return end
        pmacs.compile.run(cmdline)
      end,
    }
  end,
}

pmacs.command.define {
  name = "compile.recompile",
  description = "Re-run the last compilation with its stored command and directory.",
  fn = function()
    local last = pmacs.compile._last
    if not last then
      pmacs.editor.set_status("compile: nothing to recompile yet (run compile.run first)")
      return
    end
    pmacs.compile.run(last.cmdline, { cwd = last.cwd })
  end,
}

pmacs.command.define {
  name = "shell.command",
  description = "Run a shell command asynchronously into *shell-command* (M-!).",
  fn = function()
    pmacs.minibuffer.read {
      prompt = "Shell command: ",
      history = "shell",
      on_accept = function(cmdline)
        if cmdline == nil or cmdline == "" then return end
        pmacs.shell.command(cmdline)
      end,
    }
  end,
}

-- ---------------------------------------------------------------------
-- Global keys (Q#CM5): take over Emacs's next-error chords
-- ---------------------------------------------------------------------

-- lsp.lua bound M-g n/p to the diag commands; duplicate bindings are
-- rejected, so unbind first (this is the Q#CM1 load-order contract).
-- The dispatchers fall back to those same diag commands when nothing
-- has claimed, preserving today's behavior exactly.
pmacs.keymap.unbind { scope = "global", sequence = "M-g n" }
pmacs.keymap.unbind { scope = "global", sequence = "M-g p" }
pmacs.keymap.bind { scope = "global", sequence = "M-g n", command = "error.next" }
pmacs.keymap.bind { scope = "global", sequence = "M-g p", command = "error.previous" }
pmacs.keymap.bind { scope = "global", sequence = "C-x `", command = "error.next" }
pmacs.keymap.bind { scope = "global", sequence = "M-!", command = "shell.command" }
