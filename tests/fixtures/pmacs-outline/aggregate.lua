-- pmacs-outline/aggregate.lua --- Aggregate buffer over multiple
-- outline sources (T M8.10).
--
-- An aggregate buffer is the second buffer kind shipped by
-- outline-class. It selects matching entries from N source buffers
-- via a predicate, concatenates each entry's source-byte range
-- into a single visible buffer, and routes user edits in the
-- aggregate back to the corresponding source bytes via
-- `intercept_edit`. Source-side edits propagate back to the
-- aggregate within one async tick.
--
-- Architecture:
--
--   * Each source must already have an attached parser (the package
--     entry's `M.attach_buffer` or `parser.attach` does that for
--     outline-source buffers; aggregate.aggregate also re-attaches
--     defensively in case the caller skipped the helper). Without
--     a parser handle we can't query for matching entries.
--
--   * The aggregate's text is `concat(slice(source_i,
--     entry.byte_start, entry.byte_end) for each match)`. Each
--     "block" maps a contiguous aggregate-byte range to (source,
--     source-byte-start). Blocks are recomputed on every render.
--
--   * Write-back intercept on the aggregate routes user edits to
--     source coordinates, then returns nil so the aggregate also
--     applies the same edit locally. Repaint after source change
--     is idempotent: re-rendering produces the same text.
--
--   * Source-listener intercept on each source marks the aggregate
--     dirty and schedules an async repaint via pmacs.async +
--     pmacs.async.yield_to_next_tick(). The body yields once so the
--     source edit's Phase-3 commit completes before repaint reads
--     the source's text.
--
--   * Cross-block edits are rejected. A delete or replace whose
--     range spans more than one block --- for example, deleting
--     bytes that include the boundary between two matching
--     entries from different sources --- has no obvious mapping
--     back to source coordinates and is refused with an explicit
--     error rather than silently dropping bytes.
--
--   * Cycle detection: walks the sources list transitively
--     (following any source that's itself a registered aggregate
--     buffer); rejects if the walk revisits any buffer. v0.1's
--     immutable-sources API can't construct cycles, but the
--     detector is in place for v0.2 mutability and the test seam
--     exercises it.

local parser = require("pmacs-outline.parser")

local M = {}

-- ---------------------------------------------------------------------------
-- Handle registry
-- ---------------------------------------------------------------------------

local handles = {}

local function find_handle_by_buffer(buf)
  for _, h in ipairs(handles) do
    if h.buffer == buf then return h end
  end
  return nil
end

-- ---------------------------------------------------------------------------
-- Cycle detection
-- ---------------------------------------------------------------------------

-- Walk `sources` transitively along aggregate-dependency edges.
-- Returns true iff the walk would revisit an aggregate buffer that
-- it's currently inside (a true graph cycle through aggregates).
--
-- Pass-5 finding 3: a previous version used one global visited set
-- and treated every revisit as a cycle. That misclassified
-- duplicate plain sources --- e.g., `aggregate({S, S}, ...)` ---
-- as cycles even though duplicate sources are a legitimate use
-- (and the close path is built to handle them via paired
-- attach/detach calls). The fix: only track *aggregate* buffers
-- on the recursion stack, and only flag a cycle when the walk
-- re-enters an aggregate that's an ancestor in the current path.
-- Plain (non-aggregate) buffers can appear any number of times
-- without triggering the check.
local function would_cycle(sources, self_buffer)
  local function walk(srcs, on_path)
    for _, s in ipairs(srcs) do
      local h = find_handle_by_buffer(s)
      if h then
        if on_path[s] then return true end
        local nested = {}
        for k, v in pairs(on_path) do nested[k] = v end
        nested[s] = true
        if walk(h.sources, nested) then return true end
      end
    end
    return false
  end
  local initial = {}
  if self_buffer then initial[self_buffer] = true end
  return walk(sources, initial)
end

M.__pmacs_outline_test_would_cycle = would_cycle

-- ---------------------------------------------------------------------------
-- Render
-- ---------------------------------------------------------------------------
--
-- For each source, run parser.query with the handle's predicate;
-- for each matching entry, slice the source's [byte_start, byte_end)
-- into the aggregate. Returns the concatenated text and a blocks
-- array `{ agg_start, agg_end, source, source_start, source_end,
-- entry }`. Aggregate-byte ranges are 0-indexed and half-open
-- ([agg_start, agg_end)).

local function render(handle)
  local parts = {}
  local blocks = {}
  local agg_pos = 0
  for _, src in ipairs(handle.sources) do
    local ph = handle.parser_handles[src]
    if ph then
      local matches = parser.query(ph, handle.predicate)
      for _, entry in ipairs(matches) do
        local source_text = src:slice(entry.byte_start, entry.byte_end)
        parts[#parts + 1] = source_text
        blocks[#blocks + 1] = {
          agg_start = agg_pos,
          agg_end = agg_pos + #source_text,
          source = src,
          source_start = entry.byte_start,
          source_end = entry.byte_end,
          entry = entry,
        }
        agg_pos = agg_pos + #source_text
      end
    end
  end
  return table.concat(parts), blocks
end

-- ---------------------------------------------------------------------------
-- Painting
-- ---------------------------------------------------------------------------

local function paint(handle, text)
  handle.painting = true
  local ok, err = pcall(function()
    handle.buffer:replace(0, handle.buffer:len(), text)
  end)
  handle.painting = false
  if not ok then error(err) end
end

local function repaint(handle)
  if not handle.alive then return end
  local text, blocks = render(handle)
  handle.blocks = blocks
  paint(handle, text)
  handle.dirty = false
end

M.__pmacs_outline_test_repaint = repaint

-- ---------------------------------------------------------------------------
-- Async repaint scheduling
-- ---------------------------------------------------------------------------
--
-- Source intercepts can't repaint synchronously: the source's
-- Phase 3 hasn't committed yet, so reading source:slice would see
-- the pre-edit text. We schedule a coroutine that yields once via
-- pmacs.async.yield_to_next_tick(); the next tick_async resumes it
-- after Phase 3 has applied. `repaint_scheduled` coalesces multiple
-- source edits into one repaint per tick.
--
-- SP-7 resolution: this uses a worker-free next-tick yield primitive,
-- so propagation is pinned to the editor's async tick rather than to
-- a worker reply round trip.

local function schedule_repaint(handle)
  if handle.repaint_scheduled or not handle.alive then return end
  handle.repaint_scheduled = true
  pmacs.async(function()
    pmacs.async.yield_to_next_tick()
    handle.repaint_scheduled = false
    if handle.alive and handle.dirty then
      repaint(handle)
    end
  end)
end

-- ---------------------------------------------------------------------------
-- Write-back intercept on the aggregate
-- ---------------------------------------------------------------------------

-- Half-open `[agg_start, agg_end)` block lookup: used for the START
-- byte of delete/replace ranges. A start byte at exactly an agg_end
-- belongs to the next block (next.agg_start == prev.agg_end).
local function find_block(handle, byte)
  for _, b in ipairs(handle.blocks) do
    if byte >= b.agg_start and byte < b.agg_end then return b end
  end
  return nil
end

-- Insert-position lookup (Pass-4 finding 2): for `op.pos == agg_end
-- of a block`, append-to-end semantics map to the LATEST block
-- ending at `pos`. The strict half-open match handles inter-block
-- boundaries (next block wins by half-open semantics); the fallback
-- only fires for pos at the buffer's tail, where there is no next
-- block. This means appending at the very end of the aggregate
-- maps to inserting at `source_end` of the last matched entry,
-- which the parser then rolls into that entry's body on next reparse.
local function find_block_for_insert(handle, pos)
  for _, b in ipairs(handle.blocks) do
    if b.agg_start <= pos and pos < b.agg_end then return b end
  end
  local match = nil
  for _, b in ipairs(handle.blocks) do
    if b.agg_end == pos then match = b end
  end
  return match
end

local function block_contains_range(b, s, e)
  return s >= b.agg_start and e <= b.agg_end
end

local function map_to_source_byte(block, agg_byte)
  return block.source_start + (agg_byte - block.agg_start)
end

-- After a writeback edit applies to source S replacing the byte
-- range `[src_s, src_e)` with `delta` net bytes, update
-- handle.blocks (Pass-4 finding 3, Pass-5 finding 2, Pass-6
-- finding 1). For inserts at `pos`, callers pass `src_s = src_e =
-- pos` and `delta = bytes_len`.
--
-- Same-source bookkeeping has five cases against another block B:
--
--   (A) B.source_end <= src_s              fully before; no change.
--   (B) B.source_start >= src_e            fully after; both source
--                                          coords shift by delta.
--   (C) B.source_start <= src_s and
--       B.source_end >= src_e              B contains the edit
--                                          range; source_end shifts.
--   (D) edit range fully contains B        B's bytes were entirely
--   (E) edit partially overlaps B          deleted or replaced;
--                                          B is invalidated.
--
-- Cases D and E (Pass-6 finding 1) require dropping B from
-- handle.blocks. The deleted/replaced source bytes are gone, so
-- B's source coords no longer point at meaningful content; routing
-- a future write-back through stale B would corrupt the wrong
-- source bytes. The aggregate buffer still carries B's slice
-- (Phase 3 only deletes the user's chosen range), but those bytes
-- are zombie content that the deferred repaint will remove on its
-- next pass; until then `find_block`/`find_block_for_insert`
-- return nothing for that range, so further user edits there are
-- rejected rather than mis-routed. For inserts (src_s == src_e)
-- no block can be "inside" a zero-width range, so D/E never
-- trigger.
--
-- Pass-7 finding 1: the edited block itself follows the same
-- conservative rule. If the edit replaces/deletes the edited
-- block's full source range, the block no longer represents a
-- known matched entry. Drop it instead of leaving a zero-length
-- tail block that `find_block_for_insert` could later pick up via
-- its append-at-end fallback before the deferred repaint runs.
--
-- Aggregate-side shift is unchanged: blocks whose agg_start is at
-- or past the edited block's pre-edit agg_end shift their agg
-- coords by delta. Dropped blocks don't shift (they're gone).
local function shift_blocks_after_writeback(handle, edited_block, src_s, src_e, delta)
  if delta == 0 and src_s == src_e then return end
  local pre_agg_end = edited_block.agg_end
  local edited_invalidated =
    src_s < src_e and
    src_s <= edited_block.source_start and
    src_e >= edited_block.source_end
  local kept = {}
  for _, b in ipairs(handle.blocks) do
    if b == edited_block then
      if not edited_invalidated then
        kept[#kept + 1] = b
      end
    else
      local invalidated = false
      if b.source == edited_block.source then
        if b.source_end <= src_s then
          -- Case A: fully before.
        elseif b.source_start >= src_e then
          -- Case B: fully after.
          b.source_start = b.source_start + delta
          b.source_end = b.source_end + delta
        elseif src_s <= b.source_start and src_e >= b.source_end then
          -- Case D: edit fully contains B (subset or equal). B's
          -- bytes were entirely deleted/replaced; drop.
          invalidated = true
        elseif b.source_start < src_s and src_e < b.source_end then
          -- Case C: B *strictly* contains edit. Body grew/shrank;
          -- source_end shifts by delta.
          b.source_end = b.source_end + delta
        else
          -- Case E: partial overlap --- B's boundary is inside the
          -- edit range without strict containment in either
          -- direction. Drop.
          invalidated = true
        end
      end
      if not invalidated then
        if b.agg_start >= pre_agg_end then
          b.agg_start = b.agg_start + delta
          b.agg_end = b.agg_end + delta
        end
        kept[#kept + 1] = b
      end
    end
  end
  handle.blocks = kept
  if not edited_invalidated then
    edited_block.agg_end = edited_block.agg_end + delta
    edited_block.source_end = edited_block.source_end + delta
  end
end

local function make_writeback_intercept(handle)
  return function(op)
    if handle.painting then return nil end

    if op.kind == "insert" then
      local block = find_block_for_insert(handle, op.pos)
      if not block then
        error("pmacs-outline.aggregate: insert at byte " .. op.pos ..
              " falls outside any matched entry; aggregate edits " ..
              "must occur within a matched-entry block.")
      end
      local src_byte = map_to_source_byte(block, op.pos)
      block.source:insert(src_byte, op.bytes)
      shift_blocks_after_writeback(handle, block, src_byte, src_byte, op.bytes_len)
    elseif op.kind == "delete" then
      local s, e = op.start, op["end"]
      local block = find_block(handle, s)
      if not block or not block_contains_range(block, s, e) then
        error("pmacs-outline.aggregate: delete [" .. s .. ", " .. e ..
              ") spans block boundaries; aggregate edits must stay " ..
              "within a single matched-entry block.")
      end
      local src_s = map_to_source_byte(block, s)
      local src_e = map_to_source_byte(block, e)
      block.source:delete(src_s, src_e)
      shift_blocks_after_writeback(handle, block, src_s, src_e, -(e - s))
    elseif op.kind == "replace" then
      local s, e = op.start, op["end"]
      local block = find_block(handle, s)
      if not block or not block_contains_range(block, s, e) then
        error("pmacs-outline.aggregate: replace [" .. s .. ", " .. e ..
              ") spans block boundaries; aggregate edits must stay " ..
              "within a single matched-entry block.")
      end
      local src_s = map_to_source_byte(block, s)
      local src_e = map_to_source_byte(block, e)
      block.source:replace(src_s, src_e, op.bytes)
      shift_blocks_after_writeback(handle, block, src_s, src_e, op.bytes_len - (e - s))
    end

    handle.dirty = true
    return nil
  end
end

-- ---------------------------------------------------------------------------
-- Source-listener intercept
-- ---------------------------------------------------------------------------
--
-- Installed on each source buffer the aggregate consumes from. Runs
-- alongside the source's own parser intercept and any
-- outline-handle dirty intercept. Just marks the aggregate dirty
-- and schedules a deferred repaint.

local function make_source_listener(handle)
  return function(_op)
    handle.dirty = true
    schedule_repaint(handle)
    return nil
  end
end

-- ---------------------------------------------------------------------------
-- Public-to-package: aggregate / aggregate_close
-- ---------------------------------------------------------------------------

function M.aggregate(sources, predicate, opts)
  opts = opts or {}
  if type(sources) ~= "table" or #sources == 0 then
    error("pmacs-outline.aggregate: sources must be a non-empty array of buffers")
  end
  if type(predicate) ~= "function" then
    error("pmacs-outline.aggregate: predicate must be a function")
  end

  if would_cycle(sources, nil) then
    error("pmacs-outline.aggregate: source list contains a cycle through " ..
          "another aggregate's buffer chain.")
  end

  local agg_buf = pmacs.buffer.create(opts.name or "*outline-aggregate*")
  local handle = {
    buffer = agg_buf,
    sources = sources,
    predicate = predicate,
    parser_handles = {},
    blocks = {},
    source_listeners = {},
    painting = false,
    dirty = true,
    repaint_scheduled = false,
    alive = true,
  }

  -- Attach the parser to each source (idempotent if already attached)
  -- and install the source-listener intercept.
  for _, src in ipairs(sources) do
    handle.parser_handles[src] = parser.attach(src)
    handle.source_listeners[#handle.source_listeners + 1] =
      pmacs.buffer.add_intercept(src, make_source_listener(handle))
  end

  handle.writeback = pmacs.buffer.add_intercept(
    agg_buf, make_writeback_intercept(handle))

  handles[#handles + 1] = handle

  -- Initial render: synchronous so the aggregate is populated before
  -- aggregate() returns.
  repaint(handle)

  return handle
end

function M.aggregate_close(handle)
  handle.alive = false
  if handle.writeback then
    pcall(pmacs.buffer.remove_intercept, handle.writeback)
    handle.writeback = nil
  end
  for _, h in ipairs(handle.source_listeners) do
    pcall(pmacs.buffer.remove_intercept, h)
  end
  handle.source_listeners = {}
  -- Pass-4 finding 1: detach the parser handle for each source.
  -- We iterate `handle.sources` (not the parser_handles map) so the
  -- count of detach calls matches the count of attach calls during
  -- aggregate(): a sources list with the same buffer twice produced
  -- two attach calls and must produce two detach calls. Refcounting
  -- in parser.attach/detach ensures we don't tear down a parser
  -- that another outline view or aggregate still holds.
  for _, src in ipairs(handle.sources) do
    local ph = handle.parser_handles[src]
    if ph then parser.detach(ph) end
  end
  handle.parser_handles = {}
  if handle.buffer and pmacs.buffer.kill then
    pcall(pmacs.buffer.kill, handle.buffer)
  end
  local kept = {}
  for _, x in ipairs(handles) do
    if x ~= handle then kept[#kept + 1] = x end
  end
  handles = kept
end

-- Close every live aggregate handle (Pass-6 finding 2). Used by
-- the package's on_unload hook so a `pmacs.packages.reload` call
-- doesn't leave source-listener intercepts, parser refcounts, and
-- aggregate buffers attached after the old package's tables are
-- torn down. Safe to call multiple times --- aggregate_close
-- removes the handle from the registry.
function M.close_all_handles()
  local snapshot = {}
  for _, h in ipairs(handles) do snapshot[#snapshot + 1] = h end
  for _, h in ipairs(snapshot) do M.aggregate_close(h) end
  handles = {}
end

-- Test seam: expose the handles registry so tests can manually
-- construct cycle scenarios that the v0.1 API can't reach.
M.__pmacs_outline_test_handles = function() return handles end

return M
