-- pair.lua --- auto-pairing (Arc 2).
--
-- Typing `(` gives `()` with the cursor between; typing `)` when the
-- next char is already `)` steps over it instead of doubling it. The
-- carrier is a `buffer.after-edit` reaction (Q#AP1): the opener stays
-- a genuine single-codepoint self-insert — the classification
-- signature help depends on — and this hook inserts (or swallows) the
-- closer as a second edit. Provenance is the exact one-shot typed-edit
-- record (`pmacs.editor.take_typed_edit()`, Q#AP9), not buffer-text
-- inference: pastes, programmatic edits, manual hook runs, and a stale
-- `this_command` have no record and never pair, and a transformed,
-- relocated, or context-switching source self-insert fails closed.
--
-- This chunk loads BEFORE lsp.lua (Q#AP7): registration order is hook
-- execution order, and lsp.lua's after-edit callback synchronously
-- flushes didChange on the signature-trigger path — the closer must
-- already be in the buffer when that callback runs. Everything under
-- `pmacs.lsp` is therefore looked up lazily at callback time.
--
-- Framing: docs/auto-pairing-framing.md.

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
}

-- UTF-8 sequence length from a leading byte; nil on a continuation
-- byte (not a codepoint boundary).
local function cp_len(b)
  if b < 0x80 then return 1 end
  if b < 0xC0 then return nil end
  if b < 0xE0 then return 2 end
  if b < 0xF0 then return 3 end
  return 4
end

-- The first full UTF-8 codepoint starting at byte `pos`, as a string,
-- or nil at end-of-buffer / on a non-boundary byte. Forward twin of
-- lsp.lua's `char_before`; reads at most 4 bytes.
local function char_at(buf, pos)
  local len = buf:len()
  if pos >= len then return nil end
  local to = math.min(pos + 4, len)
  local ok, s = pcall(function() return buf:slice(pos, to) end)
  if not ok or type(s) ~= "string" or #s == 0 then return nil end
  local n = cp_len(s:byte(1))
  if not n or n > #s then return nil end
  return s:sub(1, n)
end

-- Split a pair entry into (opener, closer): EXACTLY two codepoints,
-- no trailing bytes (PR #110 round 1, finding 3 — "()x" must be
-- skipped entirely, never honored as `(` → `)x`). nil for malformed
-- user additions: skipped, not errors — the hook must never throw
-- over a config typo.
local function split_pair(s)
  if type(s) ~= "string" or #s < 2 then return nil end
  local n1 = cp_len(s:byte(1))
  if not n1 or n1 >= #s then return nil end
  local n2 = cp_len(s:byte(n1 + 1))
  if not n2 or n1 + n2 ~= #s then return nil end
  return s:sub(1, n1), s:sub(n1 + 1)
end

-- The active buffer's pair set: language entry if the language is
-- known and configured, else `default`. `pmacs.lsp` is looked up
-- lazily and nil-guarded — this chunk loads before lsp.lua (Q#AP7),
-- and language detection is an LSP-runtime service.
local function active_set()
  local lang
  if pmacs.lsp and pmacs.lsp.active_buffer_language then
    local ok, l = pcall(pmacs.lsp.active_buffer_language)
    if ok then lang = l end
  end
  return (lang and pmacs.pair.sets[lang]) or pmacs.pair.sets.default
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
-- one-shot-ness (this callback registers first and consumes it).
pmacs.pair._capture_records = false

pmacs.hook.add("buffer.after-edit", function()
  -- One-shot provenance (Q#AP9). Absence — paste, programmatic edit,
  -- manual hook run, rejected insert, a post-insert mutation by the
  -- command, stale `this_command` — is a silent non-event; only a
  -- live record for a pair-set character that then fails a gate
  -- reports.
  local rec = ed.take_typed_edit and ed.take_typed_edit()
  if pmacs.pair._capture_records then pmacs.pair._last_record = rec end
  if not rec then return end
  if not (ed.this_command and ed.this_command() == "buffer.self-insert") then return end

  local buf = pmacs.window.buffer()
  if not buf then return end

  -- Relevance first (PR #110 round 1, finding 2): pairing has no
  -- interest in characters outside the active set, so a transformed
  -- or relocated ordinary `a` must stay silent — the reports below
  -- are for pair characters only.
  local ch = rec.char
  local openers, closers = maps_for(active_set())
  if not (openers[ch] or closers[ch]) then return end

  -- Fail closed on a transformed source self-insert (Q#AP3): the
  -- intercept's positional result stands as produced; pairing on top
  -- of a relocated or expanded opener would compound it.
  if not rec.clean then
    ed.set_status("auto-pair skipped: source self-insert transformed")
    return
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
    return
  end
  -- Region guard (Q#AP3/Q#AP6): on the dispatch route type-over has
  -- already consumed and cleared the region. A region surviving the
  -- edit means the TUI's selection-blind optimistic gate let a custom
  -- pair char through (named deferral) — reacting would pile a closer
  -- onto an unconsumed region.
  if ed.region() ~= nil then return end

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
        return
      end
      if estart ~= cursor or estop ~= cursor + #ch or einserted ~= 0 then
        ed.set_status("auto-pair skip altered by buffer intercept")
        repair_cursor(win0, buf, cursor, estart, estop, einserted)
      end
      return
    end
  end

  local closer = openers[ch]
  if not closer then return end
  if not should_pair(buf, cursor, closers) then return end

  local win0 = pmacs.window.current()
  local ok, estart, estop, einserted = pcall(function()
    return buf:insert(cursor, closer)
  end)
  if not ok then
    -- Nothing landed; the opener stands alone.
    ed.set_status("auto-pair closer rejected by buffer intercept")
    return
  end
  if estart ~= cursor or estop ~= cursor or einserted ~= #closer then
    ed.set_status("auto-pair closer altered by buffer intercept")
    repair_cursor(win0, buf, cursor, estart, estop, einserted)
  end
  -- Clean path: no cursor motion — the insert landed at the cursor
  -- and Lua mutators move no cursors, so it already sits between the
  -- pair; the daemon's per-tick CursorByte re-grounds both frontends.
end)
