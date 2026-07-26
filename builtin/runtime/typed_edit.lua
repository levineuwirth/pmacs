-- typed_edit.lua --- the typed-character consumer chain (Arc 8 Stage 4a).
--
-- `pmacs.editor.take_typed_edit()` is ONE-SHOT and per-frontend (Q#AP9):
-- the first `buffer.after-edit` callback to call it clears the slot, and
-- every later callback in the same fan-out --- including a nested manual
-- `pmacs.hook.run` --- sees nil. That was survivable only because
-- auto-pairing was the sole consumer, which was never a property anyone
-- chose. A second independent caller gets nil or steals the record from
-- pairing depending on hook registration order, and registration order
-- is not a contract.
--
-- This module makes it one. It owns the single `buffer.after-edit`
-- subscriber that reads the record, and offers that one read to
-- consumers registered through `pmacs.typed_edit.add_consumer`:
--
--   pmacs.typed_edit.add_consumer {
--     name = "auto-pair",     -- for error reporting; must be unique-ish
--     priority = 100,         -- LOWEST runs FIRST
--     fn = function(rec) ... return claimed end,
--   }
--
-- A consumer returns whether it CLAIMED the edit; the first that claims
-- stops the chain. "Claimed" means the chain stops, not that an edit was
-- made --- Stage 4b's abbreviation expander claims every keystroke that
-- extends a pending abbreviation precisely so that auto-pairing does not
-- also react to it (Q#LN22).
--
-- Priority is an explicit number rather than load-order-implied, because
-- the ordering is load-bearing (Q#LN22: 64 Lean abbreviation keys
-- contain a character in the `lean4` pair set, and pairing running first
-- corrupts them) and a reader must be able to check it without
-- reconstructing `src/editor.rs`'s include list.
--
-- ORDERING CONTRACT: this chunk loads BEFORE pair.lua, which registers
-- into it, and therefore before lsp.lua. That preserves Q#AP7 --- see
-- pair.lua's header and the load site in `src/editor.rs`.
--
-- Framing: docs/lean4-mode-framing.md Q#LN10.

pmacs.typed_edit = pmacs.typed_edit or {}

-- Consumers in run order: lowest `priority` first, registration order
-- breaking ties. Maintained by ordered INSERTION rather than
-- `table.sort`, which is not stable in Lua --- equal priorities would
-- otherwise resolve arbitrarily, and "ties broken by registration
-- order" is part of the stated contract, not an incidental property.
local consumers = {}

-- Register a typed-edit consumer. Argument errors throw: registration
-- happens at chunk-load or config-load time, where a throw is a visible
-- startup failure rather than a silently missing feature. Nothing in
-- the after-edit path throws --- see the fan-out below.
function pmacs.typed_edit.add_consumer(spec)
  if type(spec) ~= "table" then
    error("pmacs.typed_edit.add_consumer: spec must be a table", 2)
  end
  local name, priority, fn = spec.name, spec.priority, spec.fn
  if type(name) ~= "string" or name == "" then
    error("pmacs.typed_edit.add_consumer: name must be a non-empty string", 2)
  end
  if type(priority) ~= "number" then
    error("pmacs.typed_edit.add_consumer: " .. name ..
      ": priority must be a number", 2)
  end
  if type(fn) ~= "function" then
    error("pmacs.typed_edit.add_consumer: " .. name ..
      ": fn must be a function", 2)
  end

  -- STRICTLY-greater comparison, so a new consumer lands AFTER every
  -- already-registered consumer of equal priority. That is exactly the
  -- registration-order tiebreak; `>=` here would silently reverse it.
  local at = #consumers + 1
  for i, c in ipairs(consumers) do
    if c.priority > priority then
      at = i
      break
    end
  end
  table.insert(consumers, at, { name = name, priority = priority, fn = fn })
end

pmacs.hook.add("buffer.after-edit", function()
  local ed = pmacs.editor
  -- ONE read for the whole fan-out (Q#AP9). The record may be nil ---
  -- paste, programmatic mutation, manual hook run, a replicated CRDT
  -- op, a stale `this_command` --- and consumers are called ANYWAY,
  -- with nil. That is deliberate: "this fan-out carried no typed edit"
  -- is information a consumer acts on. Auto-pairing's test seam
  -- observes the non-event through it, and Stage 4b abandons a pending
  -- abbreviation that an unrelated edit invalidated. Skipping the
  -- fan-out on nil would leave both reading stale state.
  local rec = ed.take_typed_edit and ed.take_typed_edit()

  for _, c in ipairs(consumers) do
    -- `buffer.after-edit` is all-must-succeed (builtin/hooks/default.lua):
    -- a throwing consumer would fail the fan-out for every OTHER
    -- subscriber, including lsp.lua's didChange flush. Contain it,
    -- report it, and keep going --- a broken consumer must not be able
    -- to stop the editor from telling the language server what changed.
    -- This matches pair.lua's existing never-throw-from-after-edit
    -- discipline; it does not weaken the hook's contract for anyone
    -- else, because the chain itself still never fails.
    local ok, claimed = pcall(c.fn, rec)
    if not ok then
      ed.set_status("typed-edit consumer '" .. c.name .. "' failed: " ..
        tostring(claimed))
    elseif claimed then
      return
    end
  end
end)
