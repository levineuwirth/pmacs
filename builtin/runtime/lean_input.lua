-- lean_input.lua --- the Lean 4 Unicode input method (Arc 8 Stage 4b).
--
-- Typing `\alpha` gives `α`; `\<>` gives `⟨⟩` with the point between.
-- The table is vendored in lean_abbrev.lua, generated from
-- vscode-lean4 — see that file's header and Q#LN11.
--
-- This is a typed-edit consumer (Stage 4a, Q#LN10) registered AHEAD of
-- auto-pairing at priority 50. The ordering is load-bearing, not
-- cosmetic: 64 abbreviation keys contain a character in the `lean4`
-- pair set (`\[[]]` → `⟦⟧`, `\{{}}` → `⦃⦄`), so with pairing first,
-- typing `\[` would insert `[]` with the point between and corrupt the
-- pending key to `\[]` before the second `[` arrives — `\[[]]` becomes
-- unreachable. Priority, not load order, is what decides this; that is
-- the whole reason Stage 4a exists.
--
-- The consumer therefore claims every keystroke that EXTENDS an open
-- pending abbreviation, not merely one that completes an expansion. A
-- consumer that claimed only completed expansions would hand each
-- intermediate `[` to pairing, which is the same corruption by a
-- different route. "Claimed" means the chain stops, not that an edit
-- was made (Q#LN22).
--
-- UNDO IS CROSS-PEER-DEGRADED, and this is accepted rather than papered
-- over (Q#LN21). `classify_key` (src/optimistic.rs) returns `Insert(c)`
-- for `\` and for every ASCII letter — only the nine built-in pair
-- chars are excluded — so on a CRDT frontend `\alpha` arrives as six
-- SOURCE-peer optimistic inserts while the expansion is a single
-- DAEMON-peer replace spanning all six. Undo across that boundary is
-- not chronologically arbitrated. This is the same defect Q#LN6 already
-- accepts for `⟨⟩`, one order of magnitude wider: it is every
-- abbreviation the user types, not a few brackets. The general fix is
-- chronological cross-peer undo arbitration, named substrate work.
-- `set_round_trip_input` would fix it and is rejected — it also makes
-- `dispatch_idle` report false, so RET would stop inserting a newline.
--
-- Framing: docs/lean4-mode-framing.md Q#LN11, Q#LN21, Q#LN22.

pmacs.lean_input = pmacs.lean_input or {}

local ed = pmacs.editor

local LEADER = "\\"
local CURSOR = "$CURSOR"

pmacs.config.define {
  name = "lean.abbrev",
  description = "Expand \\-prefixed abbreviations into Unicode symbols in Lean 4 buffers.",
  type = "boolean",
  default = true,
  mutability = "live",
}

-- ---------------------------------------------------------------------
-- The table, and the two indexes derived from it at load time
-- ---------------------------------------------------------------------

-- `best[p]` is the symbol for the shortest key having `p` as a prefix,
-- ties broken by the key's position in the vendored sequence. Both
-- halves matter: 101 prefixes have equal-shortest candidates that
-- resolve to DIFFERENT symbols (`f` → `‹` from `f<`, not `›` from
-- `f>`), and the sequence's order is the only place that tie is
-- recorded. `pairs` over a map-shaped table could not express it.
--
-- `eager[k]` marks the 1,550 keys that are complete and have no longer
-- key extending them — the ones that expand the moment they are typed,
-- with no terminator. `to` is NOT one of them (`top`, `to0`, `toa`),
-- which is exactly the case that reads as eager until the table is
-- consulted.
local best, eager = {}, {}

do
  local seq = pmacs.lean_abbrev
  if type(seq) ~= "table" then seq = {} end
  local extended = {}
  for i = 1, #seq do
    local entry = seq[i]
    local key, symbol = entry[1], entry[2]
    -- Walk every prefix of the key, including the key itself. Iterating
    -- the sequence in order and only overwriting on a STRICTLY shorter
    -- key is what makes the source-order tiebreak fall out: an equal
    -- length arriving later loses to the one already recorded.
    for n = 1, #key do
      local p = key:sub(1, n)
      local cur = best[p]
      if cur == nil or #key < cur.len then
        best[p] = { symbol = symbol, len = #key }
      end
      if n < #key then extended[p] = true end
    end
  end
  for i = 1, #seq do
    local key = seq[i][1]
    if not extended[key] then eager[key] = true end
  end
end

-- Test seam (leading underscore = not stable API). Acceptance 45g reads
-- these to pin self-consistency properties a corrupt emit would break —
-- it cannot diff against `abbreviations.json`, which is not shipped.
function pmacs.lean_input._resolve(text)
  local hit = best[text]
  return hit and hit.symbol or nil
end

function pmacs.lean_input._is_eager(key)
  return eager[key] == true
end

-- ---------------------------------------------------------------------
-- Pending state: one record per FRONTEND (Q#LN22)
-- ---------------------------------------------------------------------

-- Keyed by frontend id, with the buffer stored inside and compared by
-- value. Q#LN22 specifies the key as `(frontend, buffer)`; a per-
-- frontend slot is equivalent here and avoids inventing a scalar
-- buffer key (`BufferId`'s inner value is deliberately private, R22).
-- The generality a two-level map would add is unreachable: a frontend
-- has one point, and `buffer.after-switch` clears that frontend's slot,
-- so no frontend can hold pending state in a buffer it is not in.
--
-- Per-frontend rather than per-buffer is NOT a refinement — a buffer-
-- keyed table lets either frontend consume or discard the other's
-- half-typed abbreviation in a shared buffer, which is the ordinary
-- TUI-plus-GPU configuration this project ships.
local pending = {}

local function frontend_id()
  local ok, id = pcall(function() return pmacs.frontend.id() end)
  if ok then return id end
  return nil
end

-- Is `rec` a typed edit that continues `p` exactly? Conservative by
-- construction (Q#LN22): abandonment is LAZY because pmacs has no
-- cursor-motion hook, so every guard that would have been checked at
-- the moment the user left is checked here instead, at the next typed
-- edit.
local function still_valid(p, rec, buf)
  if p.buffer ~= rec.buffer or p.window ~= rec.window then return false end
  -- The point must still be at the end of the pending span: the leader,
  -- plus what has been typed into it, plus the character that just
  -- landed.
  if rec.effective_start ~= p.start_offset + 1 + #p.text then return false end
  -- Exactly one edit since this frontend last extended the pending
  -- abbreviation — the one being processed now. Deliberately strict
  -- across frontends: `revision()` is BUFFER-GLOBAL, so a peer editing
  -- the shared buffer invalidates this record even though it edited
  -- elsewhere. Keeping it alive would mean translating and validating
  -- the span through arbitrary peer edits, substrate Stage 4b does not
  -- add.
  local ok, rev = pcall(function() return buf:revision() end)
  if not ok or rev ~= p.expected_revision + 1 then return false end
  return true
end

-- ---------------------------------------------------------------------
-- Expansion
-- ---------------------------------------------------------------------

-- Replace the pending span with `symbol`, placing the point at
-- `$CURSOR` if the symbol carries one. Returns the byte offset just
-- past the replacement, or nil when the edit was rejected or altered.
--
-- ONE `buf:replace` for the whole expansion: one undo step, one CRDT
-- op, one effective-edit verification. A rejection drops the pending
-- state and does not retry, the same discipline as comment.lua's Q#CT5
-- and pair.lua.
local function expand(buf, p, symbol, span_end)
  local cursor_at = symbol:find(CURSOR, 1, true)
  local text = cursor_at and (symbol:gsub("%$CURSOR", "", 1)) or symbol

  local start = p.start_offset
  local ok, estart, estop, einserted = pcall(function()
    return buf:replace(start, span_end, text)
  end)
  if not ok then
    ed.set_status("lean abbreviation rejected by buffer intercept")
    return nil
  end
  if estart ~= start or estop ~= span_end or einserted ~= #text then
    ed.set_status("lean abbreviation altered by buffer intercept")
    return nil
  end

  -- The point MUST be placed explicitly. Unlike pairing's at-cursor
  -- insert, this replace SHRINKS the buffer — `\alpha` (6 bytes)
  -- becomes `α` (2) — and a point left at the pre-edit offset is past
  -- the new end. Every later self-insert is then silently rejected and
  -- the editor looks dead. There is no daemon re-grounding that covers
  -- this; that only holds for an edit that lands at the cursor.
  ed.goto_byte(cursor_at and (start + cursor_at - 1) or (start + #text))
  return start + #text
end

-- ---------------------------------------------------------------------
-- The consumer
-- ---------------------------------------------------------------------

local function on_typed_edit(rec)
  local fid = frontend_id()
  if fid == nil then return false end

  -- A fan-out carrying no record is still information: a paste,
  -- programmatic edit or replicated op landed, so whatever this
  -- frontend had pending no longer describes the buffer. Drop it and
  -- decline — this is why the chain calls consumers with nil rather
  -- than skipping them (Q#LN10).
  if not rec then
    pending[fid] = nil
    return false
  end
  if not (ed.this_command and ed.this_command() == "buffer.self-insert") then
    pending[fid] = nil
    return false
  end

  -- Both gates resolve against the SOURCE buffer of the typed edit, not
  -- the active one — a context-switching command may have replaced it
  -- by callback time (pair.lua round 2, finding 2).
  if not pmacs.config.get("lean.abbrev", rec.buffer) then
    pending[fid] = nil
    return false
  end
  local lang
  if pmacs.lsp and pmacs.lsp.buffer_language then
    local ok, l = pcall(pmacs.lsp.buffer_language, rec.buffer)
    if ok then lang = l end
  end
  if lang ~= "lean4" then
    -- No pending abbreviation is ever OPENED outside a `lean4` buffer:
    -- `\` in Rust is an ordinary character and `\[` there still pairs.
    pending[fid] = nil
    return false
  end

  local buf = pmacs.window.buffer()
  if not buf or buf ~= rec.buffer or pmacs.window.current() ~= rec.window then
    pending[fid] = nil
    return false
  end
  -- Fail closed on a transformed source self-insert, as pairing does:
  -- expanding on top of a relocated or rewritten character compounds
  -- the intercept's result.
  if not rec.clean then
    pending[fid] = nil
    return false
  end

  local revision
  do
    local ok, rev = pcall(function() return buf:revision() end)
    if not ok then
      pending[fid] = nil
      return false
    end
    revision = rev
  end

  local p = pending[fid]
  if p and not still_valid(p, rec, buf) then
    p = nil
    pending[fid] = nil
  end

  local ch = rec.char

  -- No pending abbreviation: only the leader opens one.
  if not p then
    if ch == LEADER then
      pending[fid] = {
        buffer = rec.buffer,
        window = rec.window,
        start_offset = rec.effective_start,
        text = "",
        expected_revision = revision,
      }
      -- Claimed: the leader belongs to the abbreviation, and pairing
      -- has no interest in it either way.
      return true
    end
    return false
  end

  -- Pending: does any key still have `text .. ch` as a prefix?
  local extended = p.text .. ch
  if best[extended] then
    p.text = extended
    p.expected_revision = revision
    if eager[extended] then
      local span_end = p.start_offset + 1 + #extended
      pending[fid] = nil
      expand(buf, p, best[extended].symbol, span_end)
    end
    -- Claimed either way: an extension that has not yet completed must
    -- NOT reach auto-pairing (`\[` in `\[[]]`).
    return true
  end

  -- `ch` does not extend the abbreviation. Expand what is pending
  -- FIRST, then let `ch` stand as ordinary text — the terminator is
  -- retained, not consumed, and it sits inside the replaced span so the
  -- whole thing is one undo step.
  pending[fid] = nil
  local hit = best[p.text]
  local after
  if hit and #p.text > 0 then
    -- `span_end` covers the terminator: the leader, the pending text,
    -- and `ch`, which has already landed. What replaces it is the
    -- symbol followed by `ch` itself.
    local span_end = p.start_offset + 1 + #p.text + #ch
    after = expand(buf, p, hit.symbol .. ch, span_end)
  end

  -- A terminating `\` re-arms as a NEW leader at its own position
  -- (`\alpha\to` → `α→`). Upstream gets this from `processChange`,
  -- where a finished abbreviation reports `isAffected = false` and so
  -- does not suppress the new-leader branch. This is not the `\\` case:
  -- there the pending text is empty, `\` EXTENDS, and the result is one
  -- literal backslash with no pending state left open.
  if ch == LEADER then
    local start = after and (after - #ch) or rec.effective_start
    local ok, rev = pcall(function() return buf:revision() end)
    if ok then
      pending[fid] = {
        buffer = rec.buffer,
        window = rec.window,
        start_offset = start,
        text = "",
        expected_revision = rev,
      }
    end
    return true
  end

  -- Claimed only if an expansion actually happened. Otherwise `ch` is
  -- an ordinary character in a Lean buffer and auto-pairing should see
  -- it — `\zz` leaves `z` free to pair if it ever were a pair char.
  return after ~= nil
end

-- Q#KR11's seam: a detached frontend's pending state must not outlive
-- it. Ids are monotonic, so this table would otherwise grow for the
-- life of the session.
pmacs.hook.add("frontend.detached", function(fid)
  pending[fid] = nil
end)

-- `buffer.after-switch` fires with NO arguments, so it cannot say whose
-- switch it was. The acting frontend is the one that produced the most
-- recent dispatched input event, which is what `pmacs.frontend.id()`
-- reports at callback time. Clearing every entry instead would let one
-- frontend's navigation discard another's half-typed abbreviation.
pmacs.hook.add("buffer.after-switch", function()
  local fid = frontend_id()
  if fid ~= nil then pending[fid] = nil end
end)

pmacs.typed_edit.add_consumer {
  name = "lean-abbrev",
  priority = 50,
  fn = on_typed_edit,
}
