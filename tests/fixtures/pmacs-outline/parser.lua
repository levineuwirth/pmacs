-- pmacs-outline/parser.lua --- Org-shaped headline parser with
-- lazy-incremental cache (T M8.9).
--
-- Module-internal load-bearing piece of pmacs-outline. Exposes a
-- parse-and-query surface that init.lua and nav.lua call directly,
-- and that M8.10's aggregate buffer will consume from the same
-- package without crossing the public-API boundary.
--
-- Cache policy: **lazy-reparse-on-query** (option (b) from the M8.9
-- design discussion). The intercept attached to the source buffer
-- observes edits and stashes the affected byte range as a "dirty"
-- region; reparse does not run inside the intercept. The next call
-- to `query` (or any cache-reading helper) reparses dirty regions
-- only, splices the new entries into the cache, then evaluates the
-- predicate against the now-fresh entries.
--
-- Implications:
--
--   * An edit followed by no query is free (zero reparse calls).
--   * Multiple edits between two queries coalesce into one reparse
--     per affected byte region (overlapping regions merge).
--   * The reparse boundary is the *containing subtree* of the edit,
--     extended forward to the next equal-or-higher-level headline
--     (or buffer end). Edits outside any subtree (e.g., the buffer
--     prelude before the first headline) reparse from edit-start to
--     the first headline.
--   * This *does not* satisfy edits that cross multiple subtrees
--     (e.g., a wholesale paste that contains its own headline
--     hierarchy). For those, the affected region falls back to
--     [edit_start, EOF], which is still correct (just slower).
--   * `query` evaluates the predicate against every cached entry
--     after reparse; predicates that target only one subtree still
--     reparse every dirty region (not just the targeted one). This
--     is fine for the M8.9 acceptance: the test corpus measures
--     parse-call count, not predicate-evaluation cost, and the
--     count increments by exactly one per query that finds dirty
--     regions to reparse.
--
-- Org-shape subset (per the M8.9 spec, intentionally narrow):
--
--   * Headline line: `^\*+ ` at column 0, level = number of `*`s.
--   * Trailing tags: ` :tag1:tag2:` at end of headline line; tags
--     are [A-Za-z0-9_]+, separated by `:`.
--   * Properties drawer: lines after a headline, opened by a line
--     equal to `:PROPERTIES:` and closed by `:END:`. Inside, lines
--     of the form `:KEY: value` populate the entry's properties
--     table.
--   * Body: any non-headline lines after a headline (and after
--     the closing `:END:` if a properties drawer was present).
--
-- Compatibility with arbitrary org-mode files is *not a goal*. A
-- file using only this subset parses correctly; richer org features
-- (`#+TITLE:` lines, source blocks, agenda markers, inline emphasis
-- markup) are ignored as plain body text.

local M = {}

-- ---------------------------------------------------------------------------
-- Test seam: parse-call counter
-- ---------------------------------------------------------------------------
--
-- Increments by 1 every time `parse_region` runs. The M8.9
-- acceptance test asserts the counter increments by 1 per query
-- (not per edit) when there are dirty regions, and by 0 per query
-- when there are none --- evidence of the lazy-incremental policy.
-- Loud-named per the SP-3 convention (M8.4 audit finding 7).

local parse_count = 0
M.__pmacs_outline_test_parse_count = function() return parse_count end
M.__pmacs_outline_test_reset_parse_count = function() parse_count = 0 end

-- ---------------------------------------------------------------------------
-- Region parser
-- ---------------------------------------------------------------------------
--
-- Parses a chunk of text starting at byte offset `base` (in the
-- containing buffer's coordinate space). Returns an array of
-- entries with byte ranges relative to `base`.
--
-- The region's text is taken at face value: the region is assumed
-- to begin at a fresh line boundary (start-of-buffer or just after
-- a newline). Callers ensure this by snapping the affected region
-- to a headline boundary before invoking parse_region.

local function tag_split(s)
  -- s is a colon-bracketed run like ":tag1:tag2:" with no whitespace.
  local out = {}
  local i = 2  -- skip the leading colon
  while i <= #s do
    local nx = s:find(":", i, true)
    if not nx then break end
    if nx > i then out[#out + 1] = s:sub(i, nx - 1) end
    i = nx + 1
  end
  return out
end

local function parse_headline(line)
  -- line is the raw headline text, including the leading stars and space.
  -- Returns (level, title, tags) where tags is an array (possibly empty).
  local stars = line:match("^(%*+) ")
  if not stars then return nil end
  local level = #stars
  local rest = line:sub(level + 2)  -- drop "*+ " prefix
  local tags = {}
  -- Trailing-tag pattern: at end of line, a colon-bracketed run of
  -- alphanumeric/underscore tag names separated by colons, preceded
  -- by at least one space. Lua patterns don't support `+` on groups,
  -- so we match a single colon-bracketed run --- the character class
  -- includes `:` so multiple tags joined by colons (`:tag1:tag2:`)
  -- are captured as one substring; tag_split then parses out the
  -- individual names.
  local tag_run_start, tag_run = rest:match("()%s+(:[%w_:]+:)%s*$")
  if tag_run_start then
    tags = tag_split(tag_run)
    rest = rest:sub(1, tag_run_start - 1)
  end
  -- Trim trailing whitespace from title.
  rest = rest:gsub("%s+$", "")
  return level, rest, tags
end

-- Parses `text` (the slice from `base` to `base + #text`) into an
-- array of entries with absolute byte_start / byte_end fields.
-- byte_end of each entry is the start of the next equal-or-higher
-- headline (or `base + #text` for the last subtree). headline_byte_end
-- is the byte just past the headline line's newline (or text-end if
-- there's no trailing newline).
local function parse_region(text, base)
  parse_count = parse_count + 1
  local entries = {}
  local len = #text
  local i = 1
  while i <= len do
    -- Find the end of the current line.
    local lf = text:find("\n", i, true)
    local line_end = lf and (lf - 1) or len
    local line = text:sub(i, line_end)

    if line:sub(1, 1) == "*" and parse_headline(line) then
      local level, title, tags = parse_headline(line)
      local entry = {
        byte_start = base + (i - 1),
        headline_byte_end = base + (lf or (len + 1)),
        level = level,
        title = title,
        tags = tags,
        properties = {},
      }
      -- Tag-set lookup for cheap predicate use.
      local tagset = {}
      for _, t in ipairs(tags) do tagset[t] = true end
      entry.tagset = tagset

      -- Walk forward into the body to capture an optional :PROPERTIES:
      -- drawer. Only the FIRST non-empty content block is inspected;
      -- a properties drawer must immediately follow the headline (per
      -- org convention). Body lines after the drawer or after the
      -- headline (if no drawer) are not stored --- the parser exposes
      -- the byte range; consumers slice the buffer for content.
      local probe = lf and (lf + 1) or (len + 1)
      if probe <= len then
        local pe = text:find("\n", probe, true)
        local probe_end = pe and (pe - 1) or len
        local probe_line = text:sub(probe, probe_end)
        if probe_line == ":PROPERTIES:" then
          local pi = pe and (pe + 1) or (len + 1)
          while pi <= len do
            local nx = text:find("\n", pi, true)
            local nxe = nx and (nx - 1) or len
            local pl = text:sub(pi, nxe)
            if pl == ":END:" then
              break
            end
            local key, value = pl:match("^:([%w_%-]+):%s*(.-)%s*$")
            if key then entry.properties[key] = value end
            pi = nx and (nx + 1) or (len + 1)
          end
        end
      end

      entries[#entries + 1] = entry
      i = lf and (lf + 1) or (len + 1)
    else
      -- Non-headline line; advance.
      i = lf and (lf + 1) or (len + 1)
    end
  end

  -- Compute byte_end for each entry: start of next equal-or-higher
  -- headline, or `base + len` for the last entry of the region.
  local n = #entries
  for idx = 1, n do
    local e = entries[idx]
    local end_byte = base + len
    for j = idx + 1, n do
      if entries[j].level <= e.level then
        end_byte = entries[j].byte_start
        break
      end
    end
    e.byte_end = end_byte
  end

  return entries
end

M.__pmacs_outline_test_parse_region = parse_region  -- exposed for unit tests

-- ---------------------------------------------------------------------------
-- Handle / cache
-- ---------------------------------------------------------------------------
--
-- A handle wraps a buffer and tracks the parser cache + dirty
-- regions. Created via `parser.attach(buf)` once per outline
-- buffer; stored in a module-private table keyed by buffer id.

local handles = {}  -- list, walked for is_valid sweeps

local function find_handle(buf)
  local live, found = {}, nil
  for _, h in ipairs(handles) do
    local ok, valid = pcall(h.buffer.is_valid, h.buffer)
    if ok and valid then
      live[#live + 1] = h
      if h.buffer == buf then found = h end
    else
      if h.intercept then
        pcall(pmacs.buffer.remove_intercept, h.intercept)
        h.intercept = nil
      end
    end
  end
  handles = live
  return found
end

-- Coalesce adjacent / overlapping byte ranges. Each range is
-- {start, end} in post-edit buffer coordinates. The resulting list
-- is sorted by start with no overlaps.
local function coalesce_ranges(rs)
  if #rs <= 1 then return rs end
  table.sort(rs, function(a, b) return a[1] < b[1] end)
  local out = { rs[1] }
  for i = 2, #rs do
    local last = out[#out]
    local cur = rs[i]
    if cur[1] <= last[2] then
      if cur[2] > last[2] then last[2] = cur[2] end
    else
      out[#out + 1] = cur
    end
  end
  return out
end

-- Snap an affected byte range to subtree boundaries against the
-- current cache. Returns {start, end} where:
--   * start is the byte_start of the cached entry that contains
--     `range_start`, or 0 if `range_start` falls before any
--     headline.
--   * end is the byte_end of the cached entry whose subtree extends
--     past `range_end`, or `range_end` if no later entry exists.
--
-- Subtree-aligned snapping ensures parse_region sees a chunk that
-- begins at a fresh line and contains complete headlines.
--
-- Three-stage extension to handle structural edits (Pass-3 finding 2):
--
--   1. Containing entries: if a cached entry brackets `range_start`
--      or `range_end`, snap to its boundaries (the original logic).
--   2. Backward fallback: if no entry contains `range_start`, snap
--      to the byte_end of the latest entry strictly before it (or 0
--      if none) --- the previous structural boundary.
--   3. Forward fallback: if `snap_end` is still <= `range_end`, snap
--      to the byte_start of the next entry strictly after (or
--      `total` if none) --- the next structural boundary.
--
-- Stages 2 and 3 catch the "structural merge" case: an edit that
-- deletes the boundary between two headlines (e.g., the `\n`
-- between `* A` and `* B`) drops both entries' cache, leaving the
-- dirty range surrounded by no entries; we then must reparse all
-- the way to the next surviving structural marker, which may be
-- end-of-buffer.
local function snap_to_subtree(cache, range_start, range_end, total)
  local snap_start, snap_end = 0, range_end
  local found_containing_start = false

  for _, e in ipairs(cache.entries) do
    if e.byte_start <= range_start and e.byte_end >= range_start then
      snap_start = e.byte_start
      if e.byte_end > snap_end then snap_end = e.byte_end end
      found_containing_start = true
    end
    if e.byte_start <= range_end and e.byte_end > snap_end then
      snap_end = e.byte_end
    end
  end

  if not found_containing_start then
    local backward_end = 0
    for _, e in ipairs(cache.entries) do
      if e.byte_end <= range_start and e.byte_end > backward_end then
        backward_end = e.byte_end
      end
    end
    snap_start = backward_end
  end

  if snap_end <= range_end then
    local forward_start = total
    local found = false
    for _, e in ipairs(cache.entries) do
      if e.byte_start > range_end and (not found or e.byte_start < forward_start) then
        forward_start = e.byte_start
        found = true
      end
    end
    snap_end = forward_start
  end

  return snap_start, snap_end
end

-- Recompute every entry's byte_end as the byte_start of the next
-- entry whose level is <= the entry's own level (or `total` for the
-- last subtree). This is the same derivation parse_region does at
-- the end of a parse; we run it after every shift so that snapping
-- a dirty region against the cache sees consistent byte_end values.
local function recompute_byte_ends(entries, total)
  local n = #entries
  for idx = 1, n do
    local ent = entries[idx]
    local end_byte = total
    for j = idx + 1, n do
      if entries[j].level <= ent.level then
        end_byte = entries[j].byte_start
        break
      end
    end
    ent.byte_end = end_byte
  end
end

-- Mark a byte range as dirty (post-edit coordinates); the caller has
-- already adjusted entries via the per-op update helpers.
local function record_dirty(cache, byte_start, byte_end)
  cache.dirty[#cache.dirty + 1] = { byte_start, byte_end }
  cache.dirty = coalesce_ranges(cache.dirty)
end

-- Per-op cache updaters. Each:
--   1. drops entries that intersect the edit (their content changed
--      in a way the cache can't trust without reparsing);
--   2. shifts byte_start / headline_byte_end of surviving entries
--      that begin at or after the edit endpoint;
--   3. for an entry whose subtree fully *contains* the edit (the
--      ancestor case), keeps the entry --- but its byte_end is now
--      stale and must be recomputed after the loop;
--   4. records the dirty range in post-edit coordinates;
--   5. recomputes every byte_end from the surviving byte_starts and
--      the new total buffer length.
--
-- Step 3's "contains the edit" check uses *headline_byte_end* as the
-- cutoff: an edit before headline_byte_end touches the headline line
-- itself (the title, tag run, or stars), which would invalidate
-- title/tags/level on the cached entry --- drop it. An edit at or
-- after headline_byte_end is in the body, so the headline metadata
-- survives; keep the entry. This is what the M8.9 Pass-2 finding-2
-- test asserts (deleting inside a property drawer must not lose
-- properties for the *enclosing* headline; the enclosing headline
-- survives because the edit is past its headline_byte_end).

local function update_for_insert(cache, pos, n)
  local kept = {}
  for _, ent in ipairs(cache.entries) do
    if ent.byte_start >= pos then
      -- Strictly after the insertion point: shift everything by +n.
      ent.byte_start = ent.byte_start + n
      ent.headline_byte_end = ent.headline_byte_end + n
      ent.byte_end = ent.byte_end + n
      kept[#kept + 1] = ent
    elseif ent.byte_end <= pos then
      -- Strictly before: no shift.
      kept[#kept + 1] = ent
    else
      -- Insertion is inside this entry's subtree.
      if pos < ent.headline_byte_end then
        -- Insertion within (or just before) the headline line: title
        -- /tags/level are now suspect; drop and reparse.
      else
        -- Insertion in body of this ancestor; keep, byte_end stale
        -- (will be recomputed below).
        kept[#kept + 1] = ent
      end
    end
  end
  cache.entries = kept
  -- Dirty range: cover the inserted bytes in post-edit coordinates.
  record_dirty(cache, pos, pos + n)
end

local function update_for_delete(cache, s, e)
  local n = e - s
  local kept = {}
  for _, ent in ipairs(cache.entries) do
    if ent.byte_end <= s then
      kept[#kept + 1] = ent  -- fully before
    elseif ent.byte_start >= e then
      if ent.byte_start == e then
        -- The byte at byte_start - 1 (= e - 1) was deleted. That
        -- byte was the headline line's preceding newline (or BOF
        -- alignment). The entry's line-start position is no longer
        -- structurally valid: drop and let reparse re-discover.
        -- Pass-3 finding 2.
      else
        ent.byte_start = ent.byte_start - n
        ent.headline_byte_end = ent.headline_byte_end - n
        ent.byte_end = ent.byte_end - n
        kept[#kept + 1] = ent
      end
    elseif ent.byte_start < s and ent.byte_end >= e then
      -- Deletion fully inside this entry's subtree.
      if s < ent.headline_byte_end then
        -- Deletion crosses into the headline line; drop.
      else
        -- Deletion in body; keep ancestor with stale byte_end.
        kept[#kept + 1] = ent
      end
    else
      -- Partial overlap: drop and reparse.
    end
  end
  cache.entries = kept
  record_dirty(cache, s, s)
end

local function update_for_replace(cache, s, e, n_new)
  local n_old = e - s
  local delta = n_new - n_old
  local kept = {}
  for _, ent in ipairs(cache.entries) do
    if ent.byte_end <= s then
      kept[#kept + 1] = ent
    elseif ent.byte_start >= e then
      if ent.byte_start == e then
        -- Same as delete's edge case: byte at byte_start - 1 was in
        -- the replaced range, so the entry's preceding-newline byte
        -- is now arbitrary content. Drop.
      else
        ent.byte_start = ent.byte_start + delta
        ent.headline_byte_end = ent.headline_byte_end + delta
        ent.byte_end = ent.byte_end + delta
        kept[#kept + 1] = ent
      end
    elseif ent.byte_start < s and ent.byte_end >= e then
      -- Replacement fully inside this entry's subtree.
      if s < ent.headline_byte_end then
        -- Crosses into headline; drop.
      else
        -- In body; keep ancestor with stale byte_end.
        kept[#kept + 1] = ent
      end
    else
      -- Partial overlap: drop.
    end
  end
  cache.entries = kept
  record_dirty(cache, s, s + n_new)
end

-- The intercept callback: observes edits, returns nil so the edit
-- proceeds, and updates the cache's dirty regions + byte deltas.
-- Runs *before* the rope mutates, so byte ranges in the op are in
-- pre-edit coordinates.
local function make_intercept(handle)
  return function(op)
    if handle.painting then return nil end
    local cache = handle.cache
    local total = handle.buffer:len()
    if op.kind == "insert" then
      update_for_insert(cache, op.pos, op.bytes_len)
      total = total + op.bytes_len  -- post-edit length
    elseif op.kind == "delete" then
      update_for_delete(cache, op.start, op["end"])
      total = total - (op["end"] - op.start)
    elseif op.kind == "replace" then
      update_for_replace(cache, op.start, op["end"], op.bytes_len)
      total = total + op.bytes_len - (op["end"] - op.start)
    end
    -- Recompute byte_end against the new (post-edit) total length so
    -- that the next snap_to_subtree sees a consistent cache.
    recompute_byte_ends(cache.entries, total)
    return nil
  end
end

-- Reparse all dirty regions, splicing fresh entries into the cache.
-- Increments parse_count once per region (post-coalesce). Idempotent
-- when there are no dirty regions.
local function reparse_dirty(handle)
  local cache = handle.cache
  if #cache.dirty == 0 and cache.entries_seeded then return end

  local buf = handle.buffer
  local total = buf:len()

  if not cache.entries_seeded then
    -- Initial parse: whole buffer.
    cache.entries = parse_region(buf:slice(0, total), 0)
    cache.entries_seeded = true
    cache.dirty = {}
    return
  end

  -- Snap each dirty range to subtree boundaries against the current cache,
  -- then coalesce again (snapping may extend ranges to overlap).
  local snapped = {}
  for _, r in ipairs(cache.dirty) do
    local s, e = snap_to_subtree(cache, r[1], r[2], total)
    if e > total then e = total end
    if s < 0 then s = 0 end
    snapped[#snapped + 1] = { s, e }
  end
  snapped = coalesce_ranges(snapped)

  for _, r in ipairs(snapped) do
    local s, e = r[1], r[2]
    -- Snap region forward to a headline boundary if the next cached
    -- entry beyond `e` exists with byte_start > e. We extend `e`
    -- only if a partial headline would otherwise be cut.
    local chunk = buf:slice(s, e)
    local fresh = parse_region(chunk, s)

    -- Splice: drop cached entries fully within [s, e); insert fresh.
    local kept_before, kept_after = {}, {}
    for _, ent in ipairs(cache.entries) do
      if ent.byte_end <= s then
        kept_before[#kept_before + 1] = ent
      elseif ent.byte_start >= e then
        kept_after[#kept_after + 1] = ent
      else
        -- fully or partially within: drop (parse output replaces)
      end
    end
    local merged = {}
    for _, x in ipairs(kept_before) do merged[#merged + 1] = x end
    for _, x in ipairs(fresh)        do merged[#merged + 1] = x end
    for _, x in ipairs(kept_after)   do merged[#merged + 1] = x end
    cache.entries = merged
  end

  -- After reparse, recompute byte_end for every entry: it depends on
  -- the FULL list of entries (next equal-or-higher level), so a
  -- partial reparse can't fix it locally.
  local n = #cache.entries
  for idx = 1, n do
    local ent = cache.entries[idx]
    local end_byte = total
    for j = idx + 1, n do
      if cache.entries[j].level <= ent.level then
        end_byte = cache.entries[j].byte_start
        break
      end
    end
    ent.byte_end = end_byte
  end

  cache.dirty = {}
end

-- ---------------------------------------------------------------------------
-- Public-to-package API
-- ---------------------------------------------------------------------------

-- Attach is refcounted (Pass-4 finding 1): each call to attach for
-- the same buffer returns the existing handle and bumps `refcount`;
-- each call to detach decrements; the intercept and cache are torn
-- down only when the count reaches zero. Without this, an outline
-- view and an aggregate buffer that share a source would either (a)
-- duplicate the parser intercept (extra cache work, double-fire) or
-- (b) tear down the parser when the first consumer closes, breaking
-- the second.
function M.attach(buf)
  local existing = find_handle(buf)
  if existing then
    existing.refcount = existing.refcount + 1
    return existing
  end

  local handle = {
    buffer = buf,
    cache = {
      entries = {},
      entries_seeded = false,
      dirty = {},
    },
    painting = false,
    refcount = 1,
  }
  handle.intercept = pmacs.buffer.add_intercept(buf, make_intercept(handle))
  handles[#handles + 1] = handle
  return handle
end

function M.detach(handle)
  handle.refcount = (handle.refcount or 1) - 1
  if handle.refcount > 0 then return end
  if handle.intercept then
    pcall(pmacs.buffer.remove_intercept, handle.intercept)
    handle.intercept = nil
  end
  -- Pass-5 finding 1: remove the handle from `handles` immediately
  -- when refcount drops to 0. Leaving it around lets a subsequent
  -- parser.attach(buf) for the same buffer find the dead handle,
  -- bump its refcount, and return it without reinstalling the
  -- intercept --- the cache then never invalidates on edits and
  -- queries see stale data.
  local kept = {}
  for _, h in ipairs(handles) do
    if h ~= handle then kept[#kept + 1] = h end
  end
  handles = kept
end

-- Test seam: report the current refcount for `handle` (or nil if
-- the handle has been fully torn down).
M.__pmacs_outline_test_refcount = function(handle)
  return handle.refcount
end

function M.entries(handle)
  reparse_dirty(handle)
  return handle.cache.entries
end

function M.query(handle, predicate)
  reparse_dirty(handle)
  local out = {}
  for _, e in ipairs(handle.cache.entries) do
    if predicate(e) then out[#out + 1] = e end
  end
  return out
end

function M.entry_at(handle, byte_offset)
  reparse_dirty(handle)
  for _, e in ipairs(handle.cache.entries) do
    if e.byte_start <= byte_offset and byte_offset < e.byte_end then
      -- Walk forward to find the deepest entry containing byte_offset.
      local deepest = e
      for _, candidate in ipairs(handle.cache.entries) do
        if candidate.byte_start <= byte_offset and byte_offset < candidate.byte_end
           and candidate.level > deepest.level then
          deepest = candidate
        end
      end
      return deepest
    end
  end
  return nil
end

-- Parent of `entry`: the nearest preceding entry with strictly
-- lower level. nil if `entry` is top-level.
function M.parent(handle, entry)
  reparse_dirty(handle)
  local prev = nil
  for _, e in ipairs(handle.cache.entries) do
    if e == entry then return prev end
    if e.level < entry.level then prev = e end
  end
  return nil
end

-- Next sibling-or-uncle: the nearest following entry whose level is
-- less-than-or-equal to `entry.level`. nil if none. This is the
-- "skip past subtree" target for fold-subtree.
function M.next_sibling_or_uncle(handle, entry)
  reparse_dirty(handle)
  local seen = false
  for _, e in ipairs(handle.cache.entries) do
    if seen and e.level <= entry.level then return e end
    if e == entry then seen = true end
  end
  return nil
end

-- Next entry in DFS order (any level). Used by next-headline.
function M.next_headline(handle, byte_offset)
  reparse_dirty(handle)
  for _, e in ipairs(handle.cache.entries) do
    if e.byte_start > byte_offset then return e end
  end
  return nil
end

return M
