-- pair.lua --- auto-pairing (Arc 2).
--
-- Typing `(` gives `()` with the cursor between; typing `)` when the
-- next char is already `)` steps over it instead of doubling it. The
-- carrier is a `buffer.after-edit` reaction (Q#AP1): the opener stays
-- a genuine single-codepoint self-insert — the classification
-- signature help depends on — and this reaction inserts (or swallows)
-- the closer as a second edit. Provenance is the exact one-shot
-- typed-edit record (`pmacs.editor.take_typed_edit()`, Q#AP9), not
-- buffer-text inference: pastes, programmatic edits, manual hook runs,
-- and a stale `this_command` have no record and never pair, and a
-- transformed, relocated, or context-switching source self-insert fails
-- closed.
--
-- Since Arc 8 Stage 4a (Q#LN10) pairing no longer subscribes to
-- `buffer.after-edit` itself. It registers on the typed-edit chain
-- (`builtin/runtime/typed_edit.lua`), which owns the single subscriber
-- and the single one-shot read. Everything above still holds — the
-- record is the same record — but the chain, not this file, decides
-- who sees it and in what order.
--
-- This chunk loads AFTER typed_edit.lua (it registers into it) and
-- BEFORE lsp.lua (Q#AP7): registration order is hook execution order,
-- and lsp.lua's after-edit callback synchronously flushes didChange on
-- the signature-trigger path — the closer must already be in the
-- buffer when that callback runs. Everything under `pmacs.lsp` is
-- therefore looked up lazily at callback time.
--
-- Framing: docs/archive/framings/auto-pairing-framing.md; Stage 4a in
-- docs/archive/framings/lean4-mode-framing.md Q#LN10.

pmacs.pair = pmacs.pair or {}

local ed = pmacs.editor

-- Language → array of pair strings (opener codepoint followed by
-- closer codepoint), plus the `default` entry used when the language
-- is unknown or has no entry — pairing is useful in scratch buffers
-- (Q#AP2). Public and user-extensible, like `pmacs.comment.strings`:
--   pmacs.pair.sets.rust = { "()", "[]", "{}", '""', "''" }
-- Conservative defaults: no `'` (prose apostrophes, Rust lifetimes,
-- char literals), no backtick, outside the languages that want them.
-- NOTE (Q#AP1): only the nine built-in chars `()[]{}"'` and backtick
-- are excluded from the frontends' optimistic classifiers. A
-- user-added pair char beyond those still pairs, but arrives
-- optimistically: its opener is a source-peer op and the closer a
-- daemon-peer op, so its undo is cross-peer-degraded (documented
-- limitation; the general fix is chronological cross-peer undo
-- arbitration, named substrate work).

-- Per-buffer on/off switch (Q#CR8's flagship adopter). Read against the
-- SOURCE buffer of the typed edit, never the currently active one — see
-- the consumer body below, which resolves it the same way `set_for` resolves
-- the buffer's pair set (round 2, finding 2): `rec.buffer`, not
-- `pmacs.window.buffer()`.
pmacs.config.define {
  name = "editing.auto-pair",
  description = "Automatically insert (and skip over) the closing half of a typed pair.",
  type = "boolean",
  default = true,
  mutability = "live",
}

pmacs.pair.sets = {
  default = { "()", "[]", "{}", '""' },
  python = { "()", "[]", "{}", '""', "''" },
  lua = { "()", "[]", "{}", '""', "''" },
  javascript = { "()", "[]", "{}", '""', "''", "``" },
  typescript = { "()", "[]", "{}", '""', "''", "``" },
  javascriptreact = { "()", "[]", "{}", '""', "''", "``" },
  typescriptreact = { "()", "[]", "{}", '""', "''", "``" },
  markdown = { "()", "[]", "{}", '""', "``" },
  sh = { "()", "[]", "{}", '""', "''" },
  bash = { "()", "[]", "{}", '""', "''" },
  -- Lean 4 (framing Q#LN6). `⟨⟩` (anonymous constructor) is among the
  -- most-typed constructs in Lean and omitting it would make the pair set
  -- feel broken; `⦃⦄` (strict implicit binder) and `⟮⟯` ride along because
  -- the Stage 4 input method can produce them (`\{{}}`, `\([])'`) and a
  -- bracket the pair set does not understand is worse than one it does.
  --
  -- All three are OUTSIDE the nine built-in pair chars, so per Q#AP1 their
  -- opener is a source-peer op and their closer a daemon-peer op: their undo
  -- is cross-peer-degraded. That is the documented, pre-existing limitation
  -- of user-extended pairs, whose general fix is chronological cross-peer
  -- undo arbitration (named substrate work).
  --
  -- No `''`: Lean uses `'` as a primed-identifier suffix (`h'`, `foo'`), so
  -- pairing it would fight the user constantly. Same reasoning that excludes
  -- it for Rust.
  lean4 = { "()", "[]", "{}", "⟨⟩", "⦃⦄", "⟮⟯", '""' },
}

-- Length of the well-formed UTF-8 sequence starting at `s[i]`, or nil
-- for anything ill-formed (Unicode 15, Table 3-7): continuation-byte
-- shapes are checked on EVERY trailing byte, and the narrowed
-- second-byte ranges exclude overlong encodings (C0/C1 leads,
-- E0 80–9F, F0 80–8F), UTF-16 surrogates (ED A0–BF), and codepoints
-- beyond U+10FFFF (F5+ leads, F4 90+). Length-from-lead-byte alone
-- accepted "(\xC2x" as two "codepoints" (PR #110 round 2, finding 1).
local function utf8_seq_len(s, i)
  local b1 = s:byte(i)
  if not b1 then return nil end
  if b1 < 0x80 then return 1 end
  if b1 < 0xC2 or b1 > 0xF4 then return nil end
  local b2 = s:byte(i + 1)
  if not b2 or b2 < 0x80 or b2 > 0xBF then return nil end
  if b1 < 0xE0 then return 2 end
  if b1 == 0xE0 and b2 < 0xA0 then return nil end
  if b1 == 0xED and b2 > 0x9F then return nil end
  if b1 == 0xF0 and b2 < 0x90 then return nil end
  if b1 == 0xF4 and b2 > 0x8F then return nil end
  local b3 = s:byte(i + 2)
  if not b3 or b3 < 0x80 or b3 > 0xBF then return nil end
  if b1 < 0xF0 then return 3 end
  local b4 = s:byte(i + 3)
  if not b4 or b4 < 0x80 or b4 > 0xBF then return nil end
  return 4
end

-- The first full UTF-8 codepoint starting at byte `pos`, as a string;
-- nil at end-of-buffer. Bytes that do not begin a well-formed
-- sequence (malformed file content, a truncated sequence at EOF)
-- yield the single raw byte instead: it matches neither whitespace
-- nor any validated closer, so the predicate conservatively treats
-- junk like a word character — never like EOL, which nil would mean.
local function char_at(buf, pos)
  local len = buf:len()
  if pos >= len then return nil end
  local to = math.min(pos + 4, len)
  local ok, s = pcall(function() return buf:slice(pos, to) end)
  if not ok or type(s) ~= "string" or #s == 0 then return nil end
  local n = utf8_seq_len(s, 1)
  if not n or n > #s then return s:sub(1, 1) end
  return s:sub(1, n)
end

-- Split a pair entry into (opener, closer): EXACTLY two well-formed
-- UTF-8 codepoints, no trailing bytes (PR #110 round 1 finding 3 +
-- round 2 finding 1 — "()x" and "(\xC2x" must be skipped entirely,
-- never partially honored). nil for malformed user additions:
-- skipped, not errors — the hook must never throw over a config typo.
local function split_pair(s)
  if type(s) ~= "string" or #s < 2 then return nil end
  local n1 = utf8_seq_len(s, 1)
  if not n1 or n1 >= #s then return nil end
  local n2 = utf8_seq_len(s, n1 + 1)
  if not n2 or n1 + n2 ~= #s then return nil end
  return s:sub(1, n1), s:sub(n1 + 1)
end

-- The pair set for `buf`: its language's entry if configured, else
-- `default`. Language resolves against the buffer the typed-edit
-- record names — NOT the currently active buffer, which a
-- context-switching command may have replaced by callback time
-- (PR #110 round 2, finding 2). `pmacs.lsp` is looked up lazily and
-- nil-guarded — this chunk loads before lsp.lua (Q#AP7). Non-table
-- values anywhere (a config typo like `pmacs.pair.sets.default =
-- "()"`) degrade to the default set, then to empty — never a throw
-- from the after-edit callback (round 2, finding 3).
local function set_for(buf)
  local lang
  if pmacs.lsp and pmacs.lsp.buffer_language then
    local ok, l = pcall(pmacs.lsp.buffer_language, buf)
    if ok then lang = l end
  end
  local sets = pmacs.pair.sets
  if type(sets) ~= "table" then return {} end
  local set = lang and sets[lang]
  if type(set) ~= "table" then set = sets.default end
  if type(set) ~= "table" then return {} end
  return set
end

-- opener → closer, and the set of closer codepoints.
local function maps_for(set)
  local openers, closers = {}, {}
  for _, entry in ipairs(set) do
    local o, c = split_pair(entry)
    if o then
      openers[o] = c
      closers[c] = true
    end
  end
  return openers, closers
end

-- Conservative insertion predicate (Q#AP3): pair only before
-- end-of-buffer, end-of-line, whitespace, or a closing char from the
-- active set — `foo|bar` + `(` gives `(bar`, never `()bar`.
local function should_pair(buf, cursor, closers)
  local nxt = char_at(buf, cursor)
  if nxt == nil then return true end
  if nxt == "\n" or nxt == "\r" or nxt == " " or nxt == "\t" then return true end
  return closers[nxt] == true
end

-- Right-gravity translation of `pos` through the effective edit —
-- indent.lua's repair shape (Q#AP3/Q#AP4 transformed outcomes).
local function translate(pos, estart, estop, einserted)
  if pos < estart then return pos end
  if pos > estop then return pos - (estop - estart) + einserted end
  return estart + einserted
end

-- Context-guarded cursor repair after a TRANSFORMED reaction edit:
-- the intercept's positional result stands (kind and payload are
-- immutable; the edit has already landed), so translate the pre-edit
-- cursor through the effective edit and clamp via goto_byte — unless
-- the intercept switched window or buffer, in which case the new
-- context is not ours to touch. The clean path deliberately performs
-- NO cursor motion: a clean at-cursor closer insert must leave the
-- cursor *before* the closer, which translation would not.
local function repair_cursor(win0, buf0, cursor0, estart, estop, einserted)
  if pmacs.window.current() ~= win0 or pmacs.window.buffer() ~= buf0 then
    return
  end
  ed.goto_byte(translate(cursor0, estart, estop, einserted))
end

-- Test facility (leading underscore = not stable API), OFF by
-- default: the one-shot record must stay ephemeral in production —
-- retaining every consumed record in a public field would defeat the
-- Q#AP9 contract the take API enforces (PR #110 round 1, finding 4).
-- Acceptance tests flip `_capture_records` on; each fan-out then
-- publishes the record it observed (or nil) to `_last_record`, which
-- is how tests read the exact codepoint / effective triple and prove
-- one-shot-ness (the chain takes the record before any other
-- `buffer.after-edit` subscriber can, and hands it here).
pmacs.pair._capture_records = false

-- The typed-edit consumer (Arc 8 Stage 4a, Q#LN10). `rec` is the one
-- record `typed_edit.lua` read for this fan-out — possibly nil, which
-- is why the capture seam below is updated before the nil guard.
-- Returns whether pairing CLAIMED the keystroke: true once it has
-- committed to reacting (a skip-over or a closer insert, landed or
-- intercept-rejected), false on every decline. Pairing is last of the
-- builtin consumers, so nothing currently observes that value; it is
-- stated correctly so it stays correct when something does.
local function on_typed_edit(rec)
  -- One-shot provenance (Q#AP9). Absence — paste, programmatic edit,
  -- manual hook run, rejected insert, a post-insert mutation by the
  -- command, stale `this_command` — is a silent non-event; only a
  -- live record for a pair-set character that then fails a gate
  -- reports.
  if pmacs.pair._capture_records then pmacs.pair._last_record = rec end
  if not rec then return false end
  if not (ed.this_command and ed.this_command() == "buffer.self-insert") then return false end

  -- The master switch, per-buffer (Q#CR4): the SOURCE buffer of the
  -- typed edit, resolved buffer-local -> global -> default(true). A
  -- second buffer of the same language is untouched by a buffer-local
  -- override here (acceptance 29).
  if not pmacs.config.get("editing.auto-pair", rec.buffer) then return false end

  local buf = pmacs.window.buffer()
  if not buf then return false end

  -- Relevance first (PR #110 round 1, finding 2): pairing has no
  -- interest in characters outside the set, so a transformed or
  -- relocated ordinary `a` must stay silent — the reports below are
  -- for pair characters only. The set is the SOURCE buffer's (round
  -- 2, finding 2): `'` typed in Rust stays silent even when a
  -- context-switching command lands in Python, and `'` typed in
  -- Python still draws the context-change report when it lands in
  -- Rust.
  local ch = rec.char
  local openers, closers = maps_for(set_for(rec.buffer))
  if not (openers[ch] or closers[ch]) then return false end

  -- Fail closed on a transformed source self-insert (Q#AP3): the
  -- intercept's positional result stands as produced; pairing on top
  -- of a relocated or expanded opener would compound it.
  if not rec.clean then
    ed.set_status("auto-pair skipped: source self-insert transformed")
    return false
  end
  -- Fail closed when the source edit's context is no longer current:
  -- an intercept switched window/buffer, or something moved the
  -- cursor off the post-insert position. Best-effort by construction:
  -- the report needs this fan-out to run at all, and dispatch's
  -- active-buffer revision compare (the named buffer-aware edit-epoch
  -- deferral) skips the fan-out when a context-switching command
  -- lands on a buffer with a coincidentally equal revision — pairing
  -- still fails closed there, silently (the record dies un-armed).
  if buf ~= rec.buffer
    or pmacs.window.current() ~= rec.window
    or ed.cursor() ~= rec.post_cursor then
    ed.set_status("auto-pair skipped: source context changed")
    return false
  end
  -- Region guard (Q#AP3/Q#AP6): on the dispatch route type-over has
  -- already consumed and cleared the region. A region surviving the
  -- edit means the TUI's selection-blind optimistic gate let a custom
  -- pair char through (named deferral) — reacting would pile a closer
  -- onto an unconsumed region.
  if ed.region() ~= nil then return false end

  local cursor = rec.post_cursor

  -- Skip-over-close (Q#AP4), checked before insertion so symmetric
  -- pairs (quotes) step over their own closer: typing `)` at `(|)`
  -- swallows the freshly typed duplicate, net `()` with the cursor
  -- after — exactly Emacs's skip. The pair chars round-trip (Q#AP1),
  -- so no frontend ever painted the transient duplicate.
  if closers[ch] then
    local dup_ok, dup = pcall(function() return buf:slice(cursor, cursor + #ch) end)
    if dup_ok and dup == ch then
      local win0 = pmacs.window.current()
      local ok, estart, estop, einserted = pcall(function()
        return buf:delete(cursor, cursor + #ch)
      end)
      if not ok then
        -- The duplicate stays (e.g. `())`); report, no retry.
        ed.set_status("auto-pair skip rejected by buffer intercept")
        return true
      end
      if estart ~= cursor or estop ~= cursor + #ch or einserted ~= 0 then
        ed.set_status("auto-pair skip altered by buffer intercept")
        repair_cursor(win0, buf, cursor, estart, estop, einserted)
      end
      return true
    end
  end

  local closer = openers[ch]
  if not closer then return false end
  if not should_pair(buf, cursor, closers) then return false end

  local win0 = pmacs.window.current()
  local ok, estart, estop, einserted = pcall(function()
    return buf:insert(cursor, closer)
  end)
  if not ok then
    -- Nothing landed; the opener stands alone.
    ed.set_status("auto-pair closer rejected by buffer intercept")
    return true
  end
  if estart ~= cursor or estop ~= cursor or einserted ~= #closer then
    ed.set_status("auto-pair closer altered by buffer intercept")
    repair_cursor(win0, buf, cursor, estart, estop, einserted)
  end
  -- Clean path: no cursor motion — the insert landed at the cursor
  -- and Lua mutators move no cursors, so it already sits between the
  -- pair; the daemon's per-tick CursorByte re-grounds both frontends.
  return true
end

pmacs.typed_edit.add_consumer {
  name = "auto-pair",
  priority = 100,
  fn = on_typed_edit,
}
