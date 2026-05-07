-- builtin/runtime/repl.lua --- M6.4 REPL package skeleton.
--
-- A buffer with three regions (history / prompt / input) and an
-- intercept that enforces read-only / truncate-to-input policy.
-- Spec: §sec:repl-view.
--
-- # Region tracking: marks
--
-- The history/prompt boundaries are backed by core buffer marks.
-- `_history_end` and `_prompt_end` remain as compatibility mirrors
-- for tests and package introspection, but the authoritative positions
-- are `_history_end_mark` and `_prompt_end_mark`. This matters for
-- process prompts: user edits in the input region must not accidentally
-- move the prompt boundary, while package output inserted before the
-- prompt must move both boundaries with the rope.
--
-- # Self-write bypass
--
-- The intercept enforces region read-only. The package's own writes
-- (append_output, set_prompt, submit) need to land bytes inside the
-- read-only history/prompt regions. We toggle a `_self_write` flag
-- around package-driven calls; the intercept reads it and waives the
-- policy when set. The flag is reset under pcall to survive a rope
-- error mid-write.
--
-- # Submit does not append to history
--
-- handle:submit() pops the input region's text and returns it. It
-- does NOT append the popped text to history. The shell process is
-- in raw mode and echoes user input back; the parser appends the
-- echo to history. M6.5 wires submit's return value to
-- pmacs.process.write. Doing it twice (once on submit, once on
-- echo) would double the input and force M6.5 to detect-and-suppress
-- the echo, which is the messier path. M6.4 leaves this clean for
-- M6.5 to wire.
--
-- # Process integration (M6.5)
--
-- pmacs.repl.spawn { argv, env, cwd, rows, cols, name } extends create
-- by also spawning a child via the process supervisor in raw PTY mode
-- (per spec: shell-line-editor handles its own echo, which surfaces
-- through the parser to history). The handle gains a _proc_id field
-- and is registered on a per-frame pump driven by the
-- `process.after-tick` hook. The pump drains events_take(_proc_id)
-- and routes stdout/stderr through append_output and exit events
-- through _on_exit.
--
-- Stderr handling is defensive: in PTY mode the kernel TTY layer
-- merges stderr into stdout, so stderr events should not appear at
-- all. Routing them uniformly to append_output is a safety net
-- against future regressions or pipe-mode use; seeing one in PTY
-- mode signals an upstream bug.
--
-- # Scrollback management (M6.7)
--
-- History accumulates as bytes flow in from the process. Two retention
-- knobs (lines and bytes) bound the history region; truncation drops
-- complete command-output blocks from the oldest end whenever either
-- invariant is violated. Spec: §sec:repl-perf.
--
-- Block tracking: Handle:_blocks is an array of `{ start_byte, lines }`
-- entries. The first block (start_byte = 0) covers all bytes received
-- before the first submit; this pre-first-submit block is degenerate
-- but real, so a process that produces a long preamble before its
-- first prompt still has a valid truncation boundary. Each user
-- submission opens a new block (when the current block has bytes;
-- empty submissions don't create zero-length blocks, preserving the
-- strictly-increasing start_byte invariant).
--
-- Truncation runs at tick boundaries (process.after-tick), not on
-- each byte append: per-byte overhead would regress the M6.6
-- 100 MB/s ingest gate. A `_dirty_since_last_tick` flag is set in
-- _emit_history and cleared by the truncation pass; idle handles
-- skip the check entirely. Worst-case lag from "limit exceeded" to
-- "truncation runs" is one tick (~16 ms), which is below user-
-- visible thresholds.
--
-- Single-pass truncation: when both line and byte invariants can be
-- violated, removing oldest-block-at-a-time and rechecking after each
-- removal is order-independent and always terminates. The
-- alternative (satisfy lines-first, then bytes, or vice versa) gives
-- different results when blocks have wildly different sizes; the
-- single-pass loop is the deterministic shape.

pmacs.repl = {}
local repl = pmacs.repl

-- Scrollback retention knobs. Mutable at runtime; the truncation pass
-- reads them on every check, so changing these takes effect on the
-- next tick. Defaults track §sec:repl-perf: 10000 lines is the
-- navigation/search benchmark size, and 16 MiB is the byte-pressure
-- backstop sized to keep RSS well below the M6.6 200 MB ceiling
-- across multiple concurrent REPLs.
repl.config = {
  scrollback_lines = 10000,
  scrollback_bytes = 16 * 1024 * 1024,
}

-- { [raw_proc_id_int] = handle }. Keyed on raw integer because Lua
-- raw-table-key equality does not consult ProcessIdLua's __eq. The
-- after-tick hook walks this map; spawn inserts; close removes.
local proc_pump = {}

local Handle = {}
Handle.__index = Handle

-- Toggle the self-write bypass around a closure. pcall is used so
-- the flag resets even when the wrapped call errors, otherwise a
-- single failed write would leave the bypass on for every subsequent
-- user edit.
local function with_self_write(h, fn)
  h._self_write = true
  local ok, err = pcall(fn)
  h._self_write = false
  if not ok then error(err) end
end

local function new_handle(buffer_id)
  return setmetatable({
    _buf = buffer_id,
    _parser = pmacs.ansi.parser(),
    _history_end_mark = pmacs.buffer.mark_create(buffer_id, 0, { gravity = "left" }),
    _prompt_end_mark = pmacs.buffer.mark_create(buffer_id, 0, { gravity = "left" }),
    _history_end = 0,
    _prompt_end = 0,
    -- Latest SetStyle observed. M6.4 doesn't render this anywhere
    -- (rendering will arrive with the M6.4-spec-style-channel work);
    -- we capture it so the post-alt-screen-exit running style stays
    -- consistent for M6.5.
    _current_style = nil,
    _alt_screen = false,
    _title = nil,
    _output_pos = 0,
    _capture = "history",
    _style_overlay = nil,
    _self_write = false,
    _intercept_handle = nil,
    -- Scrollback block index (M6.7). The first block is degenerate
    -- but real: it covers any bytes received before the first user
    -- submit. Subsequent blocks open in Handle:submit. The active
    -- (last) block is never removed by truncation. Each entry only
    -- carries `start_byte`; line counts are computed lazily inside
    -- the truncation pass, never on the per-byte _emit_history path
    -- (gsub allocation per byte regressed M6.6's 100 MB/s ingest
    -- gate by ~40%).
    _blocks = { { start_byte = 0 } },
    _dirty_since_last_tick = false,
  }, Handle)
end

local function sync_marks(h)
  h._history_end = h._history_end_mark:get()
  h._prompt_end = h._prompt_end_mark:get()
end

local function history_end(h)
  local pos = h._history_end_mark:get()
  h._history_end = pos
  return pos
end

local function prompt_end(h)
  local pos = h._prompt_end_mark:get()
  h._prompt_end = pos
  return pos
end

local function set_history_end(h, pos)
  h._history_end_mark:set(pos)
  h._history_end = pos
end

local function set_prompt_end(h, pos)
  h._prompt_end_mark:set(pos)
  h._prompt_end = pos
end

-- ---------------------------------------------------------------------
-- Construction / teardown
-- ---------------------------------------------------------------------

function repl.create(opts)
  opts = opts or {}
  local name = opts.name or "*repl*"
  local buf = pmacs.buffer.create(name)
  local h = new_handle(buf)
  h._intercept_handle = pmacs.buffer.add_intercept(buf, function(op)
    return repl._intercept(h, op)
  end)
  if pmacs.buffer.add_style_overlay then
    h._style_overlay = pmacs.buffer.add_style_overlay(buf)
  end
  return h
end

-- Validate argv: must be a non-empty array of strings. Returns the
-- argv unchanged on success. Errors point at the corrected call shape
-- per the project's error-message-points-at-the-workaround posture.
local function validate_argv(argv)
  if type(argv) ~= "table" then
    error("pmacs.repl.spawn: opts.argv must be an array of strings " ..
          "(e.g. argv = { \"bash\", \"-i\" })")
  end
  if #argv < 1 then
    error("pmacs.repl.spawn: opts.argv must have at least one element " ..
          "(the command); got an empty array")
  end
  for i, v in ipairs(argv) do
    if type(v) ~= "string" then
      error("pmacs.repl.spawn: opts.argv[" .. i .. "] must be a string; got "
            .. type(v))
    end
  end
  return argv
end

-- Last path component, or the whole string if there is no slash. Used
-- to derive the exit-marker name from argv[0]; users invoke `bash`,
-- not `/usr/bin/bash`. Empty input yields the empty string.
local function basename(s)
  s = s or ""
  -- Greedy `.*/` strips through the last slash, leaving the basename.
  return (s:gsub("^.*/", ""))
end

local function copy_env(env)
  local out = {}
  if env then
    for k, v in pairs(env) do out[k] = v end
  end
  return out
end

local function shell_prompt_marker_env(argv, base_env)
  local shell = basename(argv[1])
  if shell ~= "bash" and shell ~= "zsh" then
    return base_env
  end
  local env = copy_env(base_env)
  env.PS1 = "\27]133;A\7$ \27]133;B\7"
  return env
end

function repl.spawn(opts)
  opts = opts or {}
  local argv = validate_argv(opts.argv)
  local rows = opts.rows or 24
  local cols = opts.cols or 80
  local name = opts.name or ("*" .. basename(argv[1]) .. "*")

  local h = repl.create { name = name }
  h._argv = argv
  h._display_name = basename(argv[1])
  if h._display_name == "" then h._display_name = "process" end

  -- Slice argv into command + args for the supervisor's spawn shape.
  local args = {}
  for i = 2, #argv do args[i - 1] = argv[i] end

  local spec = {
    label = name,
    command = argv[1],
    args = args,
    pty = { rows = rows, cols = cols, mode = "raw" },
    ansi = true,
  }
  if opts.cwd then spec.cwd = opts.cwd end
  local env = opts.env
  if opts.prompt_markers ~= false then
    env = shell_prompt_marker_env(argv, env)
  end
  if env then spec.env = env end

  local proc_id = pmacs.process.spawn(spec)
  h._proc_id = proc_id
  proc_pump[proc_id:raw()] = h

  -- Make the REPL buffer the active window's current buffer. Without
  -- this, the buffer-scoped RET / C-c / C-d bindings never fire (the
  -- user's keys still target the previous active buffer) and the
  -- buffer-lookup commands no-op. Mirrors the convention of
  -- `pmacs.workers.show()` (see commands/default.lua:532).
  if pmacs.window and pmacs.window.switch_buffer then
    pcall(pmacs.window.switch_buffer, h._buf)
  end
  if pmacs.buffer.attach_style_overlay and h._style_overlay then
    pcall(pmacs.buffer.attach_style_overlay, h._buf, h._style_overlay)
  end

  -- Buffer-scoped bindings. RET submits the input region to the
  -- process; C-c sends SIGINT; C-d closes stdin (when input empty)
  -- or deletes the character after cursor (otherwise). Each command
  -- looks the handle up by buffer (linear scan over proc_pump; N is
  -- typically 1-3, so list-walk dominates a hash-map allocation).
  pmacs.keymap.bind {
    scope = "buffer", buffer = h._buf, sequence = "RET",
    command = "pmacs.repl.submit-current",
  }
  pmacs.keymap.bind {
    scope = "buffer", buffer = h._buf, sequence = "C-c",
    command = "pmacs.repl.send-sigint-current",
  }
  pmacs.keymap.bind {
    scope = "buffer", buffer = h._buf, sequence = "C-d",
    command = "pmacs.repl.send-eof-current",
  }

  return h
end

-- Close: send SIGTERM and let the after-tick hook drive the rest of
-- the teardown via _on_exit (which removes the handle from
-- proc_pump, clears _proc_id, and calls pmacs.process.forget).
--
-- M6.9 audit shape: close() does NOT pre-empt _on_exit's cleanup.
-- Pre-M6.9 close() eagerly cleared _proc_id and proc_pump, which
-- meant the after-tick hook stopped routing events to the closed
-- handle and the supervisor's eventual exit event was never
-- observed by the package — the supervisor retained terminated
-- process records forever (a real leak across spawn-close cycles).
-- Post-M6.9: close() sets _closing so bound commands no-op
-- immediately; the handle stays registered until _on_exit observes
-- the exit and calls forget.
function Handle:close()
  if self._intercept_handle then
    pmacs.buffer.remove_intercept(self._intercept_handle)
    self._intercept_handle = nil
  end
  if self._proc_id and not self._exited then
    self._closing = true
    -- Best-effort terminate. If the child already exited (events
    -- drained the exit event since last tick), terminate raises;
    -- pcall ignores that. _on_exit will fire on the next tick that
    -- processes the exit event.
    pcall(function() pmacs.process.terminate(self._proc_id) end)
  end
end

-- ---------------------------------------------------------------------
-- Read-only queries
-- ---------------------------------------------------------------------

function Handle:buffer_id()
  return self._buf
end

function Handle:history_end()
  return history_end(self)
end

function Handle:prompt_end()
  return prompt_end(self)
end

function Handle:title()
  return self._title
end

function Handle:input_text()
  return self._buf:slice(prompt_end(self), self._buf:len())
end

function Handle:alt_screen_active()
  return self._alt_screen
end

function Handle:style_spans()
  if not self._style_overlay then return {} end
  return self._style_overlay:spans()
end

-- ---------------------------------------------------------------------
-- Package-driven writes
-- ---------------------------------------------------------------------

-- Feed raw bytes (synthetic in M6.4, PTY in M6.5) through the ANSI
-- parser; apply each event to the buffer. Text events land at
-- history_end (extending history and pushing prompt/input forward).
-- Style events update the running style. Alt-screen markers toggle
-- suppression at the parser level (so Text events between markers
-- never reach us).
function Handle:append_output(bytes)
  self:append_events(self._parser:feed(bytes))
end

function Handle:append_events(events)
  for _, ev in ipairs(events) do
    local kind = ev.kind
    if kind == "text" then
      if self._capture == "prompt" then
        self:_emit_prompt(ev.text)
      else
        self:_emit_history(ev.text)
      end
    elseif kind == "set_style" then
      self._current_style = ev.style
    elseif kind == "alt_screen_enter" then
      self._alt_screen = true
    elseif kind == "alt_screen_exit" then
      self._alt_screen = false
    elseif kind == "set_title" then
      self._title = ev.title
    elseif kind == "prompt_start" then
      self:_begin_prompt_capture()
    elseif kind == "prompt_end" then
      self:_end_prompt_capture()
    elseif kind == "command_start" or kind == "output_start" then
      self:_begin_command_output()
    elseif kind == "carriage_return" then
      self._output_pos = self:_current_line_start()
    elseif kind == "backspace" then
      local line_start = self:_current_line_start()
      if self._output_pos > line_start then
        self._output_pos = self._output_pos - 1
      end
    elseif kind == "erase_to_eol" then
      self:_delete_history_range(self._output_pos, self:_current_line_end())
    elseif kind == "erase_line" then
      local line_start = self:_current_line_start()
      local line_end = self:_current_line_end()
      self:_delete_history_range(line_start, line_end)
      self._output_pos = line_start
    -- bracketed_paste_* markers are delimiters only; process-emitted
    -- contents are ordinary text events between them.
    end
  end
end

-- Replace the current prompt region's text. The history region is
-- untouched; the input region is preserved (it sits past prompt_end).
function Handle:set_prompt(text)
  text = text or ""
  local h_end = history_end(self)
  local p_end = prompt_end(self)
  with_self_write(self, function()
    self._buf:replace(h_end, p_end, text)
  end)
  set_prompt_end(self, history_end(self) + #text)
  sync_marks(self)
end

-- Pop the input region's text. Returns the popped string. Does NOT
-- append to history --- M6.5 echoes via the process round-trip.
--
-- M6.7: opens a new scrollback block at the current history boundary,
-- but only if the active block has accumulated bytes. Empty
-- submissions (submit-with-no-output-since-last-submit) leave the
-- block list unchanged, preserving the strictly-increasing
-- start_byte invariant.
function Handle:submit()
  local text = self:input_text()
  local p_end = prompt_end(self)
  with_self_write(self, function()
    self._buf:delete(p_end, self._buf:len())
  end)
  local last = self._blocks[#self._blocks]
  local h_end = history_end(self)
  if h_end > last.start_byte then
    self._blocks[#self._blocks + 1] = { start_byte = h_end }
  end
  return text
end

-- ---------------------------------------------------------------------
-- Internal: history extension
-- ---------------------------------------------------------------------

function Handle:_emit_history(text)
  if #text == 0 then return end
  local h_end = history_end(self)
  local pos = self._output_pos or h_end
  if pos > h_end then pos = h_end end
  local overwrite_len = math.min(#text, h_end - pos)
  local insert_len = #text - overwrite_len
  with_self_write(self, function()
    if overwrite_len > 0 then
      self._buf:replace(pos, pos + overwrite_len, text:sub(1, overwrite_len))
    end
    if insert_len > 0 then
      self._buf:insert(pos + overwrite_len, text:sub(overwrite_len + 1))
    end
  end)
  if insert_len > 0 then
    self:_adjust_blocks_after_edit(pos + overwrite_len, 0, insert_len)
  end
  sync_marks(self)
  set_history_end(self, h_end + insert_len)
  if prompt_end(self) < history_end(self) then
    set_prompt_end(self, history_end(self))
  end
  self._output_pos = pos + #text
  self:_add_style_span(pos, pos + #text)
  -- M6.7: mark the handle for the next tick's truncation check.
  -- Per-byte work beyond this assignment regresses the M6.6 100 MB/s
  -- ingest gate; line counting is deferred to _maybe_truncate.
  self._dirty_since_last_tick = true
end

function Handle:_begin_prompt_capture()
  self._capture = "prompt"
  self:set_prompt("")
end

function Handle:_emit_prompt(text)
  if #text == 0 then return end
  local p_end = prompt_end(self)
  with_self_write(self, function()
    self._buf:insert(p_end, text)
  end)
  set_prompt_end(self, p_end + #text)
  self:_add_style_span(p_end, p_end + #text)
end

function Handle:_end_prompt_capture()
  self._capture = "history"
  self._output_pos = history_end(self)
  sync_marks(self)
end

function Handle:_begin_command_output()
  self._capture = "history"
  self:set_prompt("")
  self._output_pos = history_end(self)
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

function Handle:_add_style_span(start_pos, end_pos)
  if not self._style_overlay then return end
  if start_pos >= end_pos then return end
  if style_is_default(self._current_style) then return end
  self._style_overlay:add(start_pos, end_pos, self._current_style)
end

function Handle:_adjust_blocks_after_edit(start_pos, old_len, new_len)
  local delta = new_len - old_len
  if delta == 0 then return end
  for i = 1, #self._blocks do
    local b = self._blocks[i]
    if b.start_byte > start_pos then
      b.start_byte = b.start_byte + delta
      if b.start_byte < start_pos then b.start_byte = start_pos end
    end
  end
end

function Handle:_current_line_start()
  local h_end = history_end(self)
  local pos = self._output_pos or h_end
  if pos > h_end then pos = h_end end
  local prefix = self._buf:slice(0, pos)
  local start = 0
  local search = 1
  while true do
    local idx = prefix:find("\n", search, true)
    if not idx then return start end
    start = idx
    search = idx + 1
  end
end

function Handle:_current_line_end()
  local h_end = history_end(self)
  local pos = self._output_pos or h_end
  if pos > h_end then pos = h_end end
  local suffix = self._buf:slice(pos, h_end)
  local idx = suffix:find("\n", 1, true)
  if idx then return pos + idx - 1 end
  return h_end
end

function Handle:_delete_history_range(start_pos, end_pos)
  if end_pos <= start_pos then return end
  with_self_write(self, function()
    self._buf:delete(start_pos, end_pos)
  end)
  local removed = end_pos - start_pos
  sync_marks(self)
  if self._output_pos > end_pos then
    self._output_pos = self._output_pos - removed
  elseif self._output_pos > start_pos then
    self._output_pos = start_pos
  end
  self:_adjust_blocks_after_edit(start_pos, removed, 0)
  self._dirty_since_last_tick = true
end

-- ---------------------------------------------------------------------
-- Scrollback truncation (M6.7)
-- ---------------------------------------------------------------------

-- Count newlines in `s` using string.find with the plain-text flag.
-- LuaJIT JIT-compiles this loop to memchr-equivalent speed, so it's
-- fast enough to call once per truncation pass on a multi-MB slice.
-- Not called on the hot _emit_history path.
local function count_newlines(s)
  local n = 0
  local i = 0
  while true do
    i = s:find("\n", i + 1, true)
    if not i then return n end
    n = n + 1
  end
end

-- Lazy total-lines query. Computed by scanning the rope's history
-- region; allocates one Lua string of size _history_end. Cheap
-- enough at per-tick cadence for retention sizes in the spec range
-- (16 MiB / 10000 lines), and skipped entirely by the byte-only
-- shortcut in within_limits.
local function history_lines(h)
  return count_newlines(h._buf:slice(0, history_end(h)))
end

-- Both invariants in one predicate, with a fast path that avoids the
-- expensive line count. Bound: every line is at least one byte
-- (the newline itself), so `_history_end <= scrollback_lines`
-- proves `lines <= scrollback_lines` without scanning. Same goes
-- for the byte invariant. Only when both quick checks fail do we
-- pay for the line scan.
local function within_limits(h)
  local cfg = repl.config
  local h_end = history_end(h)
  if h_end > cfg.scrollback_bytes then return false end
  if h_end <= cfg.scrollback_lines then return true end
  return history_lines(h) <= cfg.scrollback_lines
end

-- Remove the oldest scrollback block. Adjusts every position-bearing
-- field by the removed length so positions stay consistent: history
-- and prompt boundaries shift down, surviving block start_bytes
-- shift down. The buffer:delete uses the self-write bypass so the
-- read-only-history intercept doesn't veto. Caller must guarantee
-- #_blocks >= 2.
local function drop_oldest_block(h)
  local first = h._blocks[1]
  local second = h._blocks[2]
  local removed_bytes = second.start_byte - first.start_byte
  with_self_write(h, function()
    h._buf:delete(first.start_byte, second.start_byte)
  end)
  sync_marks(h)
  h._output_pos = math.max(0, (h._output_pos or history_end(h)) - removed_bytes)
  table.remove(h._blocks, 1)
  for i = 1, #h._blocks do
    h._blocks[i].start_byte = h._blocks[i].start_byte - removed_bytes
  end
end

-- Single-pass truncation. Removing oldest blocks one at a time,
-- rechecking both invariants after each removal, is order-
-- independent: the loop terminates when both hold or only the
-- active block remains. Splitting into "satisfy lines first, then
-- bytes" (or vice versa) gives different results when blocks have
-- wildly different sizes; the single-pass loop is the deterministic
-- shape and the one we want.
--
-- Fast path: with only the active block, there is nothing to drop
-- (the spec rule "removes complete command-output blocks" excludes
-- the in-progress one). Skipping the within_limits scan here keeps
-- the M6.6 stress test (no submits, one block forever) at zero
-- per-tick overhead.
function Handle:_maybe_truncate()
  if #self._blocks <= 1 then return end
  while not within_limits(self) and #self._blocks > 1 do
    drop_oldest_block(self)
  end
end

-- ---------------------------------------------------------------------
-- Intercept policy
-- ---------------------------------------------------------------------

-- Called for every apply_edit on the REPL's buffer. Returns nil
-- (pass-through), a transformed op table (truncate to input), or
-- raises (reject). Self-writes (the package's own append/set_prompt
-- /submit) bypass the policy via the _self_write flag.
function repl._intercept(h, op)
  if h._self_write then
    return nil
  end
  local prompt_end = prompt_end(h)
  if op.kind == "insert" then
    if op.pos < prompt_end then
      error("REPL: history/prompt region is read-only (insert at "
            .. op.pos .. "; input region begins at " .. prompt_end .. ")")
    end
    return nil
  elseif op.kind == "delete" then
    if op["end"] <= prompt_end then
      error("REPL: history/prompt region is read-only (delete ["
            .. op.start .. "," .. op["end"] .. "); input begins at "
            .. prompt_end .. ")")
    end
    if op.start < prompt_end then
      -- Truncate the range to the input region. Bytes are not
      -- carried by Delete ops, so this is lossless.
      return { kind = "delete", start = prompt_end, ["end"] = op["end"] }
    end
    return nil
  elseif op.kind == "replace" then
    if op["end"] <= prompt_end then
      error("REPL: history/prompt region is read-only (replace ["
            .. op.start .. "," .. op["end"] .. "); input begins at "
            .. prompt_end .. ")")
    end
    if op.start < prompt_end then
      -- Truncate the range; bytes pass through unchanged (per
      -- LuaInterceptView's M6.4 byte-immutability rule). The user's
      -- intended bytes still land at prompt_end onward; the
      -- prompt-region portion of the original range is no longer
      -- replaced. Spec: §sec:repl-view "edits that span the input
      -- region boundary are truncated to the input region."
      return { kind = "replace", start = prompt_end, ["end"] = op["end"] }
    end
    return nil
  end
end

-- ---------------------------------------------------------------------
-- Per-frame event pump (T M6.5)
-- ---------------------------------------------------------------------

-- Drain a single handle's pending supervisor events, routing each to
-- the appropriate handle method. Stdout/stderr land in append_output
-- (which feeds the parser). Exit events flag the handle and clean up
-- the registry entry. "started" / "restarting" are informational and
-- ignored by M6.5.
local function drain_handle(h)
  if not h._proc_id then return end
  local events = pmacs.process.events_take(h._proc_id)
  for _, ev in ipairs(events) do
    local kind = ev.kind
    if kind == "stdout" or kind == "stderr" then
      -- Defensive: in PTY mode the kernel TTY layer merges stderr
      -- into stdout, so stderr events should not appear here. If
      -- they do (regression / pipe-mode use), routing them through
      -- append_output preserves user output rather than dropping it.
      h:append_output(ev.bytes)
    elseif kind == "ansi" then
      h:append_events(ev.events)
    elseif kind == "exited" or kind == "signaled" or kind == "crashed" then
      h:_on_exit(ev)
    end
  end
end

-- Single subscription installed at module load. Walks the pump
-- registry and drains each handle. The hook fires every frame
-- (T M6.5 contract); an empty registry is a fast no-op.
--
-- M6.7: after draining (which may have appended bytes via
-- _emit_history → _dirty_since_last_tick = true), check truncation
-- on dirty handles. Per-tick is the right cadence: per-byte would
-- regress the M6.6 100 MB/s gate, and the worst-case lag of one
-- tick (~16 ms) is below user-visible thresholds.
pmacs.hook.add("process.after-tick", function()
  for _, h in pairs(proc_pump) do
    drain_handle(h)
    if h._dirty_since_last_tick then
      h._dirty_since_last_tick = false
      h:_maybe_truncate()
    end
  end
end)

-- Format the exit marker emitted into history when the child
-- terminates. Uses basename(argv[0]) (stored as _display_name) so
-- /usr/bin/bash displays as `bash`, falling back to `process` for
-- empty argv[0]. Always leads with `\n` so that processes which
-- exited mid-line (no trailing newline) don't run on into the
-- marker. Symbolic signal names (SIGINT, SIGTERM, ...) rather than
-- numbers, since numbers vary by platform.
local function format_exit_marker(name, ev)
  if ev.kind == "exited" then
    return string.format("\n[%s exited with code %d]\n", name, ev.code or 0)
  elseif ev.kind == "signaled" then
    return string.format("\n[%s killed by %s]\n", name, ev.signal or "signal")
  elseif ev.kind == "crashed" then
    return string.format("\n[%s crashed: %s]\n", name, ev.error or "unknown")
  else
    return string.format("\n[%s exited]\n", name)
  end
end

-- Emit the exit marker into history and finalize teardown.
-- Self-write bypass is required because the marker lands inside the
-- read-only history region. After this fires, bound commands no-op
-- (via the _exited check) and the supervisor no longer tracks the
-- process (forget releases its record).
--
-- M6.9 audit shape: _on_exit is the single point of teardown. It
-- removes the handle from proc_pump (so the after-tick hook stops
-- iterating it), clears _proc_id, and calls pmacs.process.forget so
-- the supervisor releases its terminated-process record. Pre-M6.9
-- close() eagerly cleared proc_pump and _proc_id, which prevented
-- _on_exit from firing and caused supervisor records to leak across
-- spawn-close cycles.
function Handle:_on_exit(ev)
  if self._exited then return end
  self._exited = true
  local name = self._display_name or "process"
  local marker = format_exit_marker(name, ev)
  -- _emit_history wraps the buffer write in with_self_write so the
  -- intercept lets the bytes through.
  self:_emit_history(marker)
  if self._proc_id then
    proc_pump[self._proc_id:raw()] = nil
    -- Forget releases the supervisor's record. pcall in case it has
    -- already been forgotten by the user (e.g., manual cleanup).
    pcall(pmacs.process.forget, self._proc_id)
    self._proc_id = nil
  end
end

-- ---------------------------------------------------------------------
-- Buffer-bound commands (T M6.5)
-- ---------------------------------------------------------------------

-- Find the spawned handle that owns `buf`. Linear scan over the pump
-- registry (N is typically 1-3 active REPLs). BufferIdLua's __eq
-- compares wrapped IDs, so two userdata wrapping the same buffer
-- compare equal here.
local function handle_for_buffer(buf)
  if buf == nil then return nil end
  for _, h in pairs(proc_pump) do
    if h._buf == buf then return h end
  end
  return nil
end

-- Submit the input region to the process. After-tick later routes
-- the shell's echo (or the process's plain bytes-back, for cat-style
-- programs) into history via append_output. We append "\n" so the
-- recipient sees a complete line; raw-mode shells with line editors
-- treat that as the line-end signal.
pmacs.command.define {
  name = "pmacs.repl.submit-current",
  description = "Submit the REPL input region to the spawned process.",
  fn = function()
    local h = handle_for_buffer(pmacs.window.buffer())
    if not h then return end
    if h._exited or h._closing then return end
    local text = h:submit()
    pmacs.process.write_stdin(h._proc_id, text .. "\n")
  end,
}

-- C-c: deliver SIGINT to the foreground process group. Raw-mode
-- shells (which manage their own signal handling) typically catch
-- this, abort the in-progress line, and print a fresh prompt.
pmacs.command.define {
  name = "pmacs.repl.send-sigint-current",
  description = "Send SIGINT to the spawned REPL process.",
  fn = function()
    local h = handle_for_buffer(pmacs.window.buffer())
    if not h then return end
    if h._exited or h._closing then return end
    pmacs.process.signal(h._proc_id, "INT")
  end,
}

-- C-d: spec-literal "close stdin on empty prompt", paired with
-- delete-char-forward when the input region is non-empty so users
-- never see C-d as broken. Empty case writes \x04 (EOT); raw-mode
-- shells with a line editor interpret that as end-of-input. Non-empty
-- case deletes through the REPL buffer at the cursor when it is inside
-- the input region, falling back to the input start if the editor
-- cursor is stale/outside the region.
pmacs.command.define {
  name = "pmacs.repl.send-eof-current",
  description = "Close stdin on empty input region; delete-char-forward otherwise.",
  fn = function()
    local h = handle_for_buffer(pmacs.window.buffer())
    if not h then return end
    if h._exited or h._closing then return end
    if h:input_text() == "" then
      pmacs.process.write_stdin(h._proc_id, "\x04")
    else
      local start = h:prompt_end()
      local len = h._buf:len()
      local pos = pmacs.editor.cursor()
      if pos < start or pos >= len then pos = start end
      if pos < len then h._buf:delete(pos, pos + 1) end
    end
  end,
}
