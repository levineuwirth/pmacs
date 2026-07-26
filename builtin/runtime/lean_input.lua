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

-- Expansions the chain consumer decided on but did NOT perform, keyed
-- the same way. See `run_deferred` below for why they wait.
local deferred = {}

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

-- Right-gravity translation of `pos` through the effective edit —
-- pair.lua's shape, for the same reason: the point sits AFTER the
-- replaced span (on the terminator, or on a closer pairing inserted)
-- and has to move with it.
local function translate(pos, estart, estop, einserted)
  if pos < estart then return pos end
  if pos > estop then return pos - (estop - estart) + einserted end
  return estart + einserted
end

-- Replace the pending span (leader + typed text) with `symbol`.
--
-- The span deliberately STOPS BEFORE the terminator. Including the
-- terminator would make the expansion and the terminator one edit, but
-- it would also swallow whatever auto-pairing did with that terminator
-- — and a pair character is a legal terminator (`\alp(`). One undo
-- restores the same text either way, because the terminator was its own
-- insert to begin with.
--
-- ONE `buf:replace`: one undo step, one CRDT op, one effective-edit
-- verification. A rejection drops the pending state and does not retry,
-- the same discipline as comment.lua's Q#CT5 and pair.lua.
local function expand(buf, start, span_end, symbol)
  local cursor_at = symbol:find(CURSOR, 1, true)
  local text = cursor_at and (symbol:gsub("%$CURSOR", "", 1)) or symbol

  -- The context to compare against AFTER the edit. A buffer intercept
  -- may switch window or buffer while the replace runs; the point in
  -- whatever it switched to is not ours to move.
  local win0 = pmacs.window.current()
  local point0 = ed.cursor()

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
  --
  -- Context-guarded exactly as pair.lua's `repair_cursor` is: if the
  -- intercept switched us elsewhere, `goto_byte` would move the point
  -- of a buffer that has nothing to do with this expansion.
  if pmacs.window.current() == win0 and pmacs.window.buffer() == buf then
    if cursor_at then
      ed.goto_byte(start + cursor_at - 1)
    else
      ed.goto_byte(translate(point0, estart, estop, einserted))
    end
  end
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
  -- ...and on a source edit whose context is no longer current. The
  -- buffer and window matching is not enough: a redefined self-insert
  -- can insert the character and THEN move the point, and expanding
  -- over a span the user has left teleports them back into it. Pairing
  -- makes the same three-part check for the same reason.
  if ed.cursor() ~= rec.post_cursor then
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
      pending[fid] = nil
      deferred[fid] = {
        buffer = rec.buffer,
        window = rec.window,
        start_offset = p.start_offset,
        text = extended,
        symbol = best[extended].symbol,
        re_arm = false,
      }
    end
    -- Claimed either way: an extension that has not yet completed must
    -- NOT reach auto-pairing (`\[` in `\[[]]`), and a completing one is
    -- part of the abbreviation, not a character pairing should react to.
    return true
  end

  -- `ch` does not extend the abbreviation: it TERMINATES it, and a
  -- terminator is an ordinary character that auto-pairing is entitled
  -- to react to (`\alp(` must give `α()`). So the expansion is
  -- DEFERRED to the subscriber below and this returns false, leaving
  -- pairing a record whose offsets still describe the buffer.
  --
  -- Expanding here and returning false would not do: the replace makes
  -- pairing's copy of the record stale, so pairing declines and the
  -- closer is silently lost. Expanding here and returning true is
  -- worse — it is what shipped in the first revision of this file, and
  -- it makes every pair-character terminator silently unpaired.
  pending[fid] = nil
  if best[p.text] and #p.text > 0 then
    deferred[fid] = {
      buffer = rec.buffer,
      window = rec.window,
      start_offset = p.start_offset,
      text = p.text,
      symbol = best[p.text].symbol,
      -- A terminating `\` re-arms as a NEW leader at its own position
      -- (`\al\to` → `∀→`). Upstream gets this from `processChange`,
      -- where a finished abbreviation reports `isAffected = false` and
      -- so does not suppress the new-leader branch. This is NOT the
      -- `\\` case: there the pending text is empty, `\` EXTENDS, and
      -- the result is one literal backslash with nothing left open.
      re_arm = ch == LEADER,
    }
  elseif ch == LEADER then
    -- Nothing to expand, but the leader still opens a fresh
    -- abbreviation where it landed.
    pending[fid] = {
      buffer = rec.buffer,
      window = rec.window,
      start_offset = rec.effective_start,
      text = "",
      expected_revision = revision,
    }
    return true
  end

  return false
end

-- The deferred expansion, on its own `buffer.after-edit` subscriber.
--
-- It runs AFTER the whole typed-edit chain — this chunk loads after
-- typed_edit.lua, and hook callbacks run in registration order — so
-- auto-pairing has already reacted to the terminator by the time the
-- expansion rewrites the text in front of it. Pairing's closer lands
-- after the terminator, outside the replaced span, so it survives.
--
-- It must also run BEFORE lsp.lua's subscriber (Q#AP7): that one
-- flushes `didChange` synchronously on the signature-trigger path, and
-- a server told about `\alp ` instead of `α ` stays wrong until the
-- next edit. This chunk loads before lsp.lua for exactly that reason.
--
-- A claim by ANY chain consumer stops the chain but not this — which
-- is the point. Pairing claims the terminator it reacts to.
local function run_deferred()
  local fid = frontend_id()
  if fid == nil then return end
  local d = deferred[fid]
  deferred[fid] = nil
  if not d then return end

  local buf = pmacs.window.buffer()
  if not buf or buf ~= d.buffer or pmacs.window.current() ~= d.window then
    return
  end

  -- The span must still hold exactly what was typed into it. Pairing
  -- only edits at the point, which is past this span, so in practice
  -- this holds; a buffer intercept is not obliged to be so polite.
  local span_end = d.start_offset + 1 + #d.text
  local ok, actual = pcall(function()
    return buf:slice(d.start_offset, span_end)
  end)
  if not ok or actual ~= LEADER .. d.text then return end

  local after = expand(buf, d.start_offset, span_end, d.symbol)
  if after and d.re_arm then
    local rev_ok, rev = pcall(function() return buf:revision() end)
    if rev_ok then
      pending[fid] = {
        buffer = d.buffer,
        window = d.window,
        start_offset = after,
        text = "",
        expected_revision = rev,
      }
    end
  end
end

-- Q#KR11's seam: a detached frontend's pending state must not outlive
-- it. Ids are monotonic, so this table would otherwise grow for the
-- life of the session.
pmacs.hook.add("frontend.detached", function(fid)
  pending[fid] = nil
  deferred[fid] = nil
end)

pmacs.hook.add("buffer.after-edit", run_deferred)

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
