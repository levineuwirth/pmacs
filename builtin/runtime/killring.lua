-- killring.lua --- the Emacs kill ring (Arc 2, kill-ring framing).
--
-- Kills accumulate here instead of overwriting the one clipboard slot:
-- consecutive kills append into one entry, `C-y` yanks the head, `M-y`
-- right after a yank cycles older entries. The ring is daemon-global
-- (shared across attached frontends, like the Emacs daemon); kill
-- chains and yank sessions are per-frontend, keyed by
-- `pmacs.frontend.id()` and checked against stable ring-entry ids so
-- one frontend's activity can never corrupt another's (Q#KR4/6/7).
--
-- The Rust substrate this rides on (Q#KR2): every input path either
-- rotates the per-frontend command boundary (commands) or breaks it
-- (optimistic CRDT edits, pointer gestures, pastes, unbound keys), so
-- `pmacs.editor.last_command()` is trustworthy on both frontends.
--
-- OS clipboard: the ring head is mirrored to the *acting frontend's*
-- OS clipboard on every kill/append (`ed.clipboard_set`); external
-- content joins the ring at yank time via the slot check (an OS copy
-- only reaches the daemon when pasted). `M-y` never touches the slot.
--
-- Framing: docs/archive/framings/kill-ring-framing.md.

pmacs.killring = pmacs.killring or {}

local ed = pmacs.editor

local DEFAULT_MAX = 60

local ring = {} -- array of { id, text }, most-recent first (shared)
local next_id = 1
local max_entries = DEFAULT_MAX

-- Per-frontend state (Q#KR4/KR6). Keyed by pmacs.frontend.id().
local last_kill_id = {} -- fid -> ring-entry id of that frontend's last kill
local sessions = {} -- fid -> { buffer, start, stop, entry_id, text }

-- Commands whose success may extend a kill chain (Q#KR4; the zap
-- pair joined via the editing-conveniences framing Q#EC6 — their
-- kills run inside a minibuffer accept, where the boundary state is
-- the invoking dispatch's, preserved by the minibuffer key shadow).
local KILL_CHAIN = {
  ["edit.kill-line"] = true,
  ["edit.cut"] = true,
  ["edit.zap-to-char"] = true,
  ["edit.zap-up-to-char"] = true,
}

-- Q#EC6 pending-prompt marker: fid -> true while a kill-producing
-- minibuffer prompt is armed but not yet committed. Minibuffer::begin
-- replaces a live session WITHOUT running its on_cancel, so a
-- silently-discarded zap prompt leaves no callback to break the
-- chain; the marker lives here, where every kill can see it, and an
-- ordinary kill that meets it uncommitted refuses to append.
local pending_kill_prompt = {} -- fid -> true

local function trim()
  while #ring > max_entries do
    table.remove(ring)
  end
end

-- max([n]) --- getter when nil; validated setter otherwise. Rejects
-- non-numbers, NaN, and non-finite values (math.huge would defeat the
-- cap); floors; lowering the cap trims existing entries immediately.
function pmacs.killring.max(n)
  if n == nil then return max_entries end
  if type(n) ~= "number" or n ~= n or n == math.huge or n < 1 then
    error("pmacs.killring.max: expected a finite number >= 1")
  end
  max_entries = math.floor(n)
  trim()
  return max_entries
end

-- The ring's texts, most-recent first (introspection / tests).
function pmacs.killring.list()
  local out = {}
  for i, e in ipairs(ring) do out[i] = e.text end
  return out
end

-- Test/debug seam (Q#KR11 lifecycle assertions; Q#EC6 marker).
function pmacs.killring._debug_state(fid)
  return {
    session = sessions[fid],
    last_kill_id = last_kill_id[fid],
    pending_kill_prompt = pending_kill_prompt[fid],
  }
end

-- Push `text` as a fresh entry (duplicate-of-head collapses, keeping
-- the existing id). Returns the head entry.
local function push_entry(text)
  if ring[1] and ring[1].text == text then return ring[1] end
  table.insert(ring, 1, { id = next_id, text = text })
  next_id = next_id + 1
  trim()
  return ring[1]
end

-- A kill-family command failed or was a no-op: it must not leave a
-- live chain for the next kill to append to (Q#KR4). Also drops any
-- armed-but-uncommitted prompt marker (Q#EC6): every failure path
-- routes through here, and clearing both state components together
-- is what makes break_chain sufficient.
local function fail_kill(fid)
  last_kill_id[fid] = nil
  pending_kill_prompt[fid] = nil
end

-- Chain-aware kill (Q#KR4): append to the head iff the previous
-- command was a chain kill AND this frontend's last kill IS the
-- current head (another frontend's push in between means the head is
-- not ours — append would corrupt their entry). Mirrors the head to
-- the acting frontend's OS clipboard either way.
local function kill_push(fid, text)
  -- Q#EC6 fail-safe: an uncommitted prompt marker means an armed
  -- kill prompt never resolved (silent session replacement bypasses
  -- on_cancel). Refuse to append no matter what last_command and the
  -- id say — force a fresh entry and clear the marker.
  if pending_kill_prompt[fid] then
    pending_kill_prompt[fid] = nil
    local head = push_entry(text)
    last_kill_id[fid] = head.id
    ed.clipboard_set(head.text)
    return head
  end
  local chained = KILL_CHAIN[ed.last_command() or ""]
    and last_kill_id[fid] ~= nil
    and ring[1] ~= nil
    and ring[1].id == last_kill_id[fid]
  local head
  if chained then
    ring[1].text = ring[1].text .. text
    head = ring[1]
  else
    head = push_entry(text)
  end
  last_kill_id[fid] = head.id
  ed.clipboard_set(head.text)
  return head
end

-- edit.cut body (C-w): kill the active region into the ring.
function pmacs.killring.cut()
  local fid = pmacs.frontend.id()
  local region = ed.region()
  local buf = pmacs.window.buffer()
  if not region or not buf then
    fail_kill(fid)
    ed.set_status("no region")
    return false
  end
  local rstart, rstop = region.start, region["end"]
  local text = buf:slice(rstart, rstop)
  -- Same intercept discipline as kill_line, via the mutator so the
  -- EFFECTIVE edit is checkable exactly. The selection is cleared
  -- explicitly (ed.delete_region did that as a side effect).
  local dok, estart, estop, einserted = pcall(function()
    return buf:delete(rstart, rstop)
  end)
  ed.clear_selection()
  if not dok then
    fail_kill(fid)
    ed.set_status("kill rejected by buffer intercept")
    return false
  end
  if estart ~= rstart or estop ~= rstop or einserted ~= 0 then
    fail_kill(fid)
    ed.set_status("kill altered by buffer intercept; ring not updated")
    return false
  end
  ed.goto_byte(rstart)
  kill_push(fid, text)
  return true
end

-- edit.copy body (M-w): save the region to the ring without deleting.
-- Not a chain command (Q#KR4's family is kill-line + cut): a copy
-- pushes fresh (duplicate-of-head collapses) and neither extends nor
-- starts an append chain.
function pmacs.killring.copy()
  local fid = pmacs.frontend.id()
  local region = ed.region()
  local buf = pmacs.window.buffer()
  if not region or not buf then
    fail_kill(fid)
    ed.set_status("no region")
    return false
  end
  local text = buf:slice(region.start, region["end"])
  push_entry(text)
  ed.clipboard_set(text)
  fail_kill(fid) -- a copy is not an appendable kill
  return true
end

-- edit.kill-line body (C-k): kill from the cursor to end of line; at
-- the newline itself, kill the newline (Emacs kill-line with
-- kill-whole-line nil). Consecutive C-k's append (Q#KR4), so
-- C-k C-k C-k builds one multi-line entry.
function pmacs.killring.kill_line()
  local fid = pmacs.frontend.id()
  local buf = pmacs.window.buffer()
  if not buf then
    fail_kill(fid)
    return false
  end
  local cursor = ed.cursor()
  local len = buf:len()
  if cursor >= len then
    fail_kill(fid)
    ed.set_status("end of buffer")
    return false
  end
  -- Find the next newline by chunked scan (lines are almost always
  -- shorter than one chunk; a chunk loop keeps giant lines safe).
  local eol = nil
  local p = cursor
  while p < len do
    local chunk_to = math.min(p + 4096, len)
    local chunk = buf:slice(p, chunk_to)
    local nl = chunk:find("\n", 1, true)
    if nl then
      eol = p + nl - 1
      break
    end
    p = chunk_to
  end
  local kill_to
  if eol == cursor then
    kill_to = cursor + 1 -- at the newline: kill the newline itself
  else
    kill_to = eol or len -- rest of the line (or of a final bare line)
  end
  local text = buf:slice(cursor, kill_to)
  -- The delete runs the buffer's edit intercepts, which may REJECT
  -- (error) or TRANSFORM the operation. A rejection must clear the
  -- kill chain (or the next C-k would append to a kill that never
  -- happened); a transformation means the bytes actually removed are
  -- not `text`, so pushing `text` would put never-killed bytes on the
  -- ring and the OS clipboard. The mutators return the EFFECTIVE edit
  -- (post-intercept start/end/inserted), so this is an exact check —
  -- a length delta would be defeated by an equal-length rewrite to a
  -- different range.
  local ok, estart, estop, einserted = pcall(function()
    return buf:delete(cursor, kill_to)
  end)
  if not ok then
    fail_kill(fid)
    ed.set_status("kill rejected by buffer intercept")
    return false
  end
  if estart ~= cursor or estop ~= kill_to or einserted ~= 0 then
    fail_kill(fid)
    ed.set_status("kill altered by buffer intercept; ring not updated")
    return false
  end
  kill_push(fid, text)
  return true
end

-- Drop a frontend's yank session (invalid M-y must not leave state a
-- second M-y could ride, Q#KR7).
local function drop_session(fid, msg)
  sessions[fid] = nil
  if msg then ed.set_status(msg) end
end

-- edit.paste body (C-y): yank the ring head (Q#KR6).
function pmacs.killring.yank()
  local fid = pmacs.frontend.id()
  -- Slot check: content that arrived via an OS paste (paste_inbound
  -- refreshes the slot) joins the ring the first time it is yanked.
  local slot = ed.clipboard_get()
  if slot and slot ~= "" and (not ring[1] or ring[1].text ~= slot) then
    push_entry(slot)
  end
  local head = ring[1]
  if not head then
    drop_session(fid, "kill ring empty")
    return false
  end
  local region = ed.region()
  local start = region and region.start or ed.cursor()
  if not slot or slot ~= head.text then
    ed.clipboard_set(head.text)
  end
  local ok = ed.clipboard_paste()
  if not ok then
    drop_session(fid) -- failed paste creates no session (Q#KR6)
    return false
  end
  local buf = pmacs.window.buffer()
  sessions[fid] = {
    buffer = buf and tostring(buf) or "",
    start = start,
    stop = ed.cursor(),
    entry_id = head.id,
    text = head.text,
  }
  return true
end

-- edit.yank-pop body (M-y): replace the just-yanked text with the
-- next-older ring entry (Q#KR7). Valid only immediately after a yank
-- or another pop, with a live, still-verifiable session.
function pmacs.killring.yank_pop()
  local fid = pmacs.frontend.id()
  local lc = ed.last_command()
  local s = sessions[fid]
  if not (lc == "edit.paste" or lc == "edit.yank-pop") or not s then
    drop_session(fid, "previous command was not a yank")
    return false
  end
  local buf = pmacs.window.buffer()
  if not buf or tostring(buf) ~= s.buffer then
    drop_session(fid, "yank was in another buffer")
    return false
  end
  -- Invalidation guard: the remembered range must still hold exactly
  -- the text this session yanked. A concurrent edit (another
  -- frontend, a hook) that moved or altered it fails here — refuse
  -- rather than splice garbage. The slice is pcall'd: an upstream
  -- deletion can shrink the buffer below `stop`, and an out-of-bounds
  -- range must read as "changed", not throw.
  local ok, current = pcall(function() return buf:slice(s.start, s.stop) end)
  if not ok or current ~= s.text then
    drop_session(fid, "buffer changed since the yank")
    return false
  end
  -- Stable-id rotation: find where this session's entry sits NOW
  -- (other frontends' pushes shift positions, not ids) and step to
  -- the next older, wrapping. An evicted id invalidates.
  local pos = nil
  for i, e in ipairs(ring) do
    if e.id == s.entry_id then
      pos = i
      break
    end
  end
  if not pos then
    drop_session(fid, "kill ring entry expired")
    return false
  end
  local entry = ring[pos % #ring + 1]
  -- The replace runs buffer intercepts, which may REJECT (error) or
  -- TRANSFORM. A rejection must still end the session — letting the
  -- error propagate would leave `sessions[fid]` live for a second M-y
  -- to reuse. And the verification must be EXACT: the mutator returns
  -- the effective edit, and any deviation from the requested
  -- (start, stop, #text) — e.g. an intercept enlarging `stop` by one
  -- byte, silently deleting extra content — ends the session (the
  -- interceptor's result stands; accepted post-hoc semantics, Q#KR7).
  local rok, estart, estop, einserted = pcall(function()
    return buf:replace(s.start, s.stop, entry.text)
  end)
  if not rok then
    drop_session(fid, "yank-pop rejected by buffer intercept")
    return false
  end
  if estart ~= s.start or estop ~= s.stop or einserted ~= #entry.text then
    drop_session(fid, "yank-pop altered by buffer intercept; stopped")
    return false
  end
  ed.goto_byte(s.start + #entry.text)
  s.stop = s.start + #entry.text
  s.entry_id = entry.id
  s.text = entry.text
  return true
end

-- ---- editing-conveniences exports (Q#EC6) --------------------------
-- The chain-aware surface zap needs from its minibuffer accept: a
-- range kill with killring's exact-effective-edit discipline, a
-- targeted chain break (the frontend whose chain must break is the
-- INVOKING one, which need not be the acting one), and the
-- pending-prompt marker lifecycle.

-- Kill [start, stop) of the ACTIVE buffer into the ring, chain-aware.
-- Returns true on a clean kill; false, "rejected" when an intercept
-- threw (nothing landed); false, "transformed", estart, estop,
-- einserted when the effective edit deviated (the intercept's result
-- stands — the caller owns any cursor repair). Both failure paths
-- break the acting frontend's chain and report status. Invalid
-- arguments error BEFORE any ring or buffer mutation: misuse of a
-- programmatic API, not a user outcome.
function pmacs.killring.kill_range(start, stop)
  local buf = pmacs.window.buffer()
  if not buf then
    error("pmacs.killring.kill_range: no active buffer")
  end
  local function ok_int(n)
    return type(n) == "number" and n == n and n ~= math.huge
      and n == math.floor(n) and n >= 0
  end
  if not (ok_int(start) and ok_int(stop))
    or start >= stop or stop > buf:len() then
    error("pmacs.killring.kill_range: expected integers "
      .. "0 <= start < stop <= buffer length")
  end
  local fid = pmacs.frontend.id()
  local text = buf:slice(start, stop)
  local ok, estart, estop, einserted = pcall(function()
    return buf:delete(start, stop)
  end)
  if not ok then
    fail_kill(fid)
    ed.set_status("kill rejected by buffer intercept")
    return false, "rejected"
  end
  if estart ~= start or estop ~= stop or einserted ~= 0 then
    fail_kill(fid)
    ed.set_status("kill altered by buffer intercept; ring not updated")
    return false, "transformed", estart, estop, einserted
  end
  kill_push(fid, text)
  return true
end

-- Public chain break. Targets `fid` when given (the origin guard
-- passes the INVOKING frontend, which may differ from the acting
-- one), else the acting frontend.
function pmacs.killring.break_chain(fid)
  if fid == nil then
    fid = pmacs.frontend.id()
  elseif type(fid) ~= "number" or fid ~= fid or fid == math.huge
    or fid ~= math.floor(fid) or fid < 0 then
    error("pmacs.killring.break_chain: fid must be a nonnegative integer")
  end
  fail_kill(fid)
end

-- Arm the acting frontend's pending-prompt marker (zap, at invoke
-- time, before minibuffer.read). Does NOT touch last_kill_id —
-- backward chaining (C-k then a completed zap appends) needs the id
-- alive. An ALREADY-set marker means the previous armed prompt was
-- silently discarded without resolution (replaced session, no
-- on_cancel): break that chain first, or a second zap would commit
-- the stale marker away and falsely append to the pre-abandonment
-- kill.
function pmacs.killring.arm_kill_prompt()
  local fid = pmacs.frontend.id()
  if pending_kill_prompt[fid] then
    last_kill_id[fid] = nil
  end
  pending_kill_prompt[fid] = true
end

-- Clear the acting frontend's marker, reporting whether one was
-- still armed. Callers kill only on true: a false return means some
-- other Lua consumed the marker while the prompt was open, and the
-- armed state is no longer trustworthy (fail closed).
function pmacs.killring.commit_kill_prompt()
  local fid = pmacs.frontend.id()
  local was_armed = pending_kill_prompt[fid] ~= nil
  pending_kill_prompt[fid] = nil
  return was_armed
end

-- Q#KR11: a detached frontend's chain/session state must not outlive
-- it (ids are monotonic; these tables would grow forever).
pmacs.hook.add("frontend.detached", function(fid)
  sessions[fid] = nil
  last_kill_id[fid] = nil
  pending_kill_prompt[fid] = nil
end)

pmacs.command.define {
  name = "edit.kill-line",
  description = "Kill from the cursor to the end of the line (into the kill ring).",
  fn = function() pmacs.killring.kill_line() end,
}

pmacs.command.define {
  name = "edit.yank-pop",
  description = "Replace the just-yanked text with the previous kill (after C-y).",
  fn = function() pmacs.killring.yank_pop() end,
}

pmacs.keymap.bind { scope = "global", sequence = "C-k", command = "edit.kill-line" }
pmacs.keymap.bind { scope = "global", sequence = "M-y", command = "edit.yank-pop" }
