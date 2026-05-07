-- pmacs-magit/fold.lua --- Foldable-section primitive (T M8.5).
--
-- This module is the load-bearing piece of pmacs-magit: parsing a
-- caller-provided section spec into a tree, rendering it as plain
-- text, and projecting it through a fold-state map. M8.6 and M8.7
-- (Git status integration, gestures) build on this primitive.
--
-- The module is intentionally self-contained --- it has no
-- dependency on `pmacs.*` Lua APIs and no side effects. It works
-- on plain Lua tables and strings. That makes it
--
--   * unit-testable from the same Lua VM the test fixture uses,
--   * promotable to a shared package layer if M8.6 finds the
--     pattern wants it (per the M11 substance-floor logic --- the
--     audit decides; speculation here is just a clean module
--     boundary, not an API contract).
--
-- Public surface (all under the returned table):
--
--   parse(spec)              -> flat array of sections (DFS preorder)
--   render_source(flat)      -> { text = ..., line_index = ... }
--   render_visible(flat, fs) -> { text = ..., line_index = ...,
--                                 fold_targets = ... }
--   section_at(line_idx, projection)
--                            -> section id at that visible line
--   toggle(fold_state, id)   -> mutates fold_state in place
--
-- "spec" is a tree:
--
--   { id = "a", title = "Section A", body = "line1\nline2",
--     children = {
--       { id = "a1", title = "Subsection A1", body = "..." },
--     },
--   }
--
-- Either the top-level spec is a single root or an array of roots.
-- IDs must be unique across the tree (validated at parse time).
-- Body is optional; nil-or-empty-string means a header-only
-- section.

local M = {}

-- ---------------------------------------------------------------------------
-- Parse
-- ---------------------------------------------------------------------------

local function is_array(t)
  return type(t) == "table" and (t[1] ~= nil or next(t) == nil)
end

local function copy_body_lines(body)
  if body == nil or body == "" then return {} end
  if type(body) ~= "string" then
    error("section body must be a string; got " .. type(body))
  end
  local lines = {}
  -- Match Lua's standard newline-split: empty strings between
  -- consecutive newlines are preserved; a trailing newline does
  -- *not* generate a phantom empty line.
  local i = 1
  while i <= #body + 1 do
    local nl = body:find("\n", i, true)
    if nl then
      lines[#lines + 1] = body:sub(i, nl - 1)
      i = nl + 1
    else
      lines[#lines + 1] = body:sub(i)
      break
    end
  end
  -- A body that ends with \n produced one trailing empty entry; drop it.
  if body:sub(-1) == "\n" and lines[#lines] == "" then
    lines[#lines] = nil
  end
  return lines
end

local function visit(node, depth, parent_id, out, seen_ids)
  if type(node) ~= "table" then
    error("section spec entries must be tables; got " .. type(node))
  end
  if type(node.id) ~= "string" or node.id == "" then
    error("section spec missing string id (got " .. tostring(node.id) .. ")")
  end
  if seen_ids[node.id] then
    error("duplicate section id '" .. node.id .. "'")
  end
  seen_ids[node.id] = true
  if type(node.title) ~= "string" then
    error("section '" .. node.id .. "' missing string title")
  end
  out[#out + 1] = {
    id = node.id,
    parent_id = parent_id,
    depth = depth,
    title = node.title,
    body_lines = copy_body_lines(node.body),
    -- We compute child_ids in a second pass so the parent knows
    -- what to skip when collapsed.
    child_ids = {},
  }
  local self_idx = #out
  if node.children ~= nil then
    if not is_array(node.children) then
      error("section '" .. node.id .. "' children must be an array")
    end
    for _, c in ipairs(node.children) do
      visit(c, depth + 1, node.id, out, seen_ids)
      out[self_idx].child_ids[#out[self_idx].child_ids + 1] = c.id
    end
  end
end

-- Parse a user spec into a flat array of sections in DFS preorder.
-- The array's order *is* the source-buffer order: section i appears
-- before section j when i precedes j in this list. Each entry:
--
--   { id, parent_id, depth, title, body_lines, child_ids }
--
-- depth starts at 0 for top-level sections.
function M.parse(spec)
  if type(spec) ~= "table" then
    error("section spec must be a table; got " .. type(spec))
  end
  local out = {}
  local seen_ids = {}
  -- Accept either a single root or an array of roots. An empty
  -- array is allowed at the parse layer (returns no sections);
  -- callers that require at least one section enforce that on
  -- their side so the error names the calling API rather than
  -- the parser.
  if is_array(spec) and spec[1] ~= nil then
    for _, node in ipairs(spec) do
      visit(node, 0, nil, out, seen_ids)
    end
  elseif is_array(spec) then
    -- empty array: zero roots; out stays empty.
  else
    visit(spec, 0, nil, out, seen_ids)
  end
  return out
end

-- ---------------------------------------------------------------------------
-- Render
-- ---------------------------------------------------------------------------

-- Produce the indented header text for a section. The depth-1
-- indent is two spaces; the header carries no fold marker because
-- the source must be fold-invariant. Visible projection adds the
-- marker.
local function header_line_source(section)
  return string.rep("  ", section.depth) .. section.title
end

-- Visible header includes a fold marker: "v " for expanded
-- (foldable), "> " for collapsed (foldable), "  " for non-foldable
-- (no body, no children --- the marker would be misleading because
-- there's nothing to fold). Foldable = has body OR has children;
-- a body-only leaf is foldable because its body genuinely hides.
local function header_line_visible(section, foldable, expanded)
  local marker
  if not foldable then
    marker = "  "
  elseif expanded then
    marker = "v "
  else
    marker = "> "
  end
  return string.rep("  ", section.depth) .. marker .. section.title
end

local function is_foldable(section)
  return #section.child_ids > 0 or #section.body_lines > 0
end

local function body_line(section, raw)
  -- Body indented one further level than the header.
  return string.rep("  ", section.depth + 1) .. raw
end

-- Render the source text: every section's header + body, every
-- child, depth-first, regardless of fold state. Returns
--   { text = "...", line_index = { [line_no] = section_id } }
-- where line_no is 0-indexed and points to either a header or a
-- body line; in the latter case the section_id is the section
-- whose body it belongs to.
function M.render_source(flat)
  local lines = {}
  local line_index = {}
  for _, s in ipairs(flat) do
    lines[#lines + 1] = header_line_source(s)
    line_index[#lines - 1] = s.id
    for _, b in ipairs(s.body_lines) do
      lines[#lines + 1] = body_line(s, b)
      line_index[#lines - 1] = s.id
    end
  end
  return { text = table.concat(lines, "\n"), line_index = line_index }
end

-- Render the visible projection. fold_state is a table mapping
-- section_id to one of: nil / "expanded" (visible) / "collapsed"
-- (header shown, body + descendants hidden). Returns
--   {
--     text = "...",
--     line_index = { [line_no] = section_id },
--     fold_targets = { [line_no] = section_id }, -- only header lines
--   }
-- The fold_targets table is the lookup the toggle-fold command
-- uses: a cursor on a header line maps to the section it heads;
-- a cursor on a body line maps to the section whose body it is
-- (toggle on body folds the parent header too --- magit
-- semantics).
function M.render_visible(flat, fold_state)
  fold_state = fold_state or {}
  local lines = {}
  local line_index = {}
  local fold_targets = {}

  -- Pre-build a "skip until depth <= K" pointer using DFS-preorder
  -- properties: children of a collapsed section come immediately
  -- after it in `flat`, until we hit a section at the collapsed
  -- section's own depth (or shallower).
  local i = 1
  local n = #flat
  while i <= n do
    local s = flat[i]
    local foldable = is_foldable(s)
    local expanded = (fold_state[s.id] ~= "collapsed")

    -- Header line for s.
    lines[#lines + 1] = header_line_visible(s, foldable, expanded)
    line_index[#lines - 1] = s.id
    fold_targets[#lines - 1] = s.id

    if expanded then
      -- Body lines belong to s; cursor on a body line should
      -- toggle s's fold (consistent with magit's treatment of
      -- "section under point").
      for _, b in ipairs(s.body_lines) do
        lines[#lines + 1] = body_line(s, b)
        line_index[#lines - 1] = s.id
        fold_targets[#lines - 1] = s.id
      end
      i = i + 1  -- continue with children, if any
    else
      -- Skip body and descendants: jump i past every entry whose
      -- depth is greater than s.depth.
      i = i + 1
      while i <= n and flat[i].depth > s.depth do
        i = i + 1
      end
    end
  end

  return {
    text = table.concat(lines, "\n"),
    line_index = line_index,
    fold_targets = fold_targets,
  }
end

-- Look up the section id under a given visible line. Returns nil
-- if the line is out of range (cursor past end of buffer).
function M.section_at(visible_projection, line_no)
  return visible_projection.fold_targets[line_no]
end

-- Toggle fold state for a section id. nil/expanded -> collapsed;
-- collapsed -> expanded. Mutates the table in place. Body-only
-- leaves are foldable because collapsing them hides their body;
-- true leaves (no body, no children) can carry a tracked state, but
-- rendering them is unchanged because there is nothing to hide.
function M.toggle(fold_state, id)
  if fold_state[id] == "collapsed" then
    fold_state[id] = "expanded"
  else
    fold_state[id] = "collapsed"
  end
end

return M
