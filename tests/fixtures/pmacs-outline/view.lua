-- pmacs-outline/view.lua --- Visible projection rendering (T M8.9).
--
-- Builds a read-only projection buffer from a source outline buffer
-- + per-handle fold state. Folding visually replaces a folded
-- entry's body bytes with a `...` marker; selective rendering
-- derives entirely from the parsed structure.
--
-- The projection is *byte-substitution*, not a structural rebuild:
-- for each top-level folded subtree (folded entries that are not
-- inside another folded ancestor's hidden range), the source bytes
-- from `headline_byte_end` to `byte_end` are replaced with the
-- collapse marker; everything outside those ranges appears
-- verbatim. This keeps the projection text close to the source
-- text, so cursor navigation in the visible buffer maps back to
-- source byte ranges via the line index built during render.

local M = {}

-- Top-level folded subtrees: walk entries in DFS order, skip any
-- entry whose ancestor (in byte-range terms) is already folded.
-- Each emitted range is `{ headline_byte_end, byte_end }` of the
-- folded entry.
local function top_level_folded(entries, fold_state)
  local out = {}
  local active_end = -1  -- byte_end of the outermost active fold
  for _, e in ipairs(entries) do
    if e.byte_start < active_end then
      -- Inside an already-folded ancestor: skip.
    else
      active_end = -1
      if fold_state[e.byte_start] then
        out[#out + 1] = { e.headline_byte_end, e.byte_end, e }
        active_end = e.byte_end
      end
    end
  end
  return out
end

-- Build the visible text + a line_to_byte map (1-indexed line
-- numbers in the visible buffer -> byte offsets in the source). The
-- map only records the byte offset of the *start* of each visible
-- line; intermediate columns are not tracked.
function M.render(source_text, entries, fold_state, marker)
  marker = marker or "  ...\n"
  local folds = top_level_folded(entries, fold_state)
  table.sort(folds, function(a, b) return a[1] < b[1] end)

  local parts = {}
  local cursor = 0  -- 0-indexed byte cursor into source_text
  for _, f in ipairs(folds) do
    local fold_start, fold_end = f[1], f[2]
    if fold_start > cursor then
      parts[#parts + 1] = source_text:sub(cursor + 1, fold_start)
    end
    parts[#parts + 1] = marker
    cursor = fold_end
  end
  if cursor < #source_text then
    parts[#parts + 1] = source_text:sub(cursor + 1)
  end
  local text = table.concat(parts)

  -- visible_to_source: per visible line, the source byte offset of
  -- that line's first character. Built by walking visible text and
  -- tracking corresponding source offsets through the substitutions.
  local visible_to_source = { 0 }
  local src_idx = 0
  local fold_idx = 1
  local i = 1
  while i <= #text do
    local nl = text:find("\n", i, true)
    if not nl then break end
    -- Advance src_idx by the same number of bytes from `i` to `nl`,
    -- but jump source offsets across folded ranges.
    local visible_consumed = nl - i + 1
    local k = i
    while k <= nl do
      -- If we're at the start of the marker, jump source past the fold.
      if fold_idx <= #folds and src_idx == folds[fold_idx][1]
         and text:sub(k, k + #marker - 1) == marker then
        src_idx = folds[fold_idx][2]
        fold_idx = fold_idx + 1
        k = k + #marker
      else
        src_idx = src_idx + 1
        k = k + 1
      end
    end
    visible_to_source[#visible_to_source + 1] = src_idx
    i = nl + 1
  end

  return {
    text = text,
    visible_to_source = visible_to_source,
    folds = folds,
  }
end

-- Find the byte offset in the *source* buffer corresponding to a
-- visible-buffer line number (0-indexed, matching pmacs's
-- cursor_line()). Out-of-range lines map to source EOF.
function M.source_byte_at_visible_line(projection, line_0indexed)
  local map = projection.visible_to_source
  local idx = line_0indexed + 1  -- map is 1-indexed
  if idx < 1 then return 0 end
  if idx > #map then return map[#map] end
  return map[idx]
end

-- Inverse: given a source byte offset, find the visible line that
-- shows it (or the line of the fold marker that hides it).
function M.visible_line_at_source_byte(projection, source_byte)
  local map = projection.visible_to_source
  -- Walk forward until map[i+1] > source_byte; line is i-1 (0-indexed).
  for i = 1, #map do
    if map[i] > source_byte then return i - 2 end
  end
  return #map - 1
end

return M
