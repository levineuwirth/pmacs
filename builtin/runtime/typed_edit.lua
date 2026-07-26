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
--   local handle = pmacs.typed_edit.add_consumer {
--     name = "auto-pair",     -- for error reporting
--     priority = 100,         -- LOWEST runs FIRST
--     fn = function(rec) ... return claimed end,
--   }
--   pmacs.typed_edit.remove_consumer(handle)  -- -> true if it was live
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

-- Handles are opaque to callers; only identity matters. An integer
-- counter is enough because nothing ever reuses one.
local next_handle = 0

-- `math.huge` is the only portable spelling of infinity available in
-- both LuaJIT and 5.4, and NaN is the only value not equal to itself.
local INT32_MIN, INT32_MAX = -2147483648, 2147483647

-- Register a typed-edit consumer; returns an opaque handle for
-- `remove_consumer`. Argument errors throw: registration happens at
-- chunk-load or config-load time, where a throw is a visible startup
-- failure rather than a silently missing feature. Nothing in the
-- after-edit path throws --- see the fan-out below.
function pmacs.typed_edit.add_consumer(spec)
  if type(spec) ~= "table" then
    error("pmacs.typed_edit.add_consumer: spec must be a table", 2)
  end
  local name, priority, fn = spec.name, spec.priority, spec.fn
  if type(name) ~= "string" or name == "" then
    error("pmacs.typed_edit.add_consumer: name must be a non-empty string", 2)
  end
  -- A bare `type(priority) == "number"` admits NaN and the infinities,
  -- and EVERY ordered comparison against NaN is false --- so a NaN
  -- consumer silently lands wherever the insertion scan happens to give
  -- up, and the lowest-first contract other consumers depend on stops
  -- holding. Bounded integers match `pmacs.completion.register`, whose
  -- priority is an i32 on the Rust side.
  if type(priority) ~= "number" or priority ~= priority
    or priority == math.huge or priority == -math.huge
    or priority % 1 ~= 0
    or priority < INT32_MIN or priority > INT32_MAX then
    error("pmacs.typed_edit.add_consumer: " .. name ..
      ": priority must be a finite integer in [-2147483648, 2147483647]", 2)
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
  next_handle = next_handle + 1
  local handle = next_handle
  table.insert(consumers, at,
    { handle = handle, name = name, priority = priority, fn = fn })
  return handle
end

-- Unregister a consumer by the handle `add_consumer` returned. Returns
-- true if it was registered, false otherwise (so a double-remove is a
-- reportable no-op rather than a throw). Without this, re-evaluating a
-- config or reloading a package accumulates callbacks permanently ---
-- the leak COHERENCE.md §13 already records against `pmacs.hook.add`,
-- which this chain would otherwise inherit and spread.
function pmacs.typed_edit.remove_consumer(handle)
  for i, c in ipairs(consumers) do
    if c.handle == handle then
      table.remove(consumers, i)
      return true
    end
  end
  return false
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

  -- Iterate a SNAPSHOT. A consumer may register or remove consumers
  -- while the chain is running, and `table.insert`/`table.remove` on
  -- the live array shifts indices under `ipairs` --- a consumer that
  -- registers a lower-priority one shifts itself forward and runs
  -- twice, and repeating that is unbounded. Registrations and removals
  -- made during a fan-out therefore take effect on the NEXT fan-out.
  local snapshot = {}
  for i, c in ipairs(consumers) do
    snapshot[i] = c
  end

  for _, c in ipairs(snapshot) do
    -- Each consumer gets its OWN copy of the record. The table handed
    -- out is plain Lua data, so a declining consumer could otherwise
    -- edit `rec.char` in place and the next consumer would act on the
    -- forged value --- auto-pairing reads `rec.char` to decide what to
    -- close, so a rewritten `char` makes it insert a pair the user
    -- never typed. Every field is a scalar or an opaque id, so a
    -- shallow copy is a complete snapshot.
    local mine = nil
    if rec ~= nil then
      mine = {}
      for k, v in pairs(rec) do
        mine[k] = v
      end
    end

    -- Contain the consumer. A throw here would skip every LATER
    -- consumer in the chain and mark the whole `buffer.after-edit` run
    -- failed; the other subscribers still run, because all-must-succeed
    -- collects errors and continues (`src/hook.rs`'s
    -- `run_all_must_succeed`), but one broken consumer must not be able
    -- to silently disable the ones behind it. This matches pair.lua's
    -- existing never-throw-from-after-edit discipline.
    local ok, claimed = pcall(c.fn, mine)
    if not ok then
      -- Rendering is itself protected: a Lua error may be any value,
      -- including a table whose `__tostring` throws, and an escaping
      -- error here would defeat the containment above.
      local shown, rendered = pcall(tostring, claimed)
      if not shown or type(rendered) ~= "string" then
        rendered = "<unprintable error>"
      end
      pcall(ed.set_status,
        "typed-edit consumer '" .. c.name .. "' failed: " .. rendered)
    elseif claimed then
      return
    end
  end
end)
