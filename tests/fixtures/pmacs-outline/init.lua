-- pmacs-outline/init.lua --- Outline structure parser & view (T M8.9).
--
-- The third of M8's three universality-proof packages, after
-- dired-class (M8.2-M8.4) and magit-class (M8.5-M8.8). Validates
-- the *parsed-structure-from-rope* shape: a source buffer holds the
-- raw outline text; a paired visible buffer is a read-only
-- projection through fold state; selective rendering and folding
-- both derive from the parsed structure.
--
-- Architecture (T M8.9 --- two-buffer projection, M8.5 magit-class
-- precedent):
--
--   * source buffer: caller-owned, editable. Holds the canonical
--     outline text. The parser observes edits via an intercept
--     and maintains a lazy-incremental cache (parser.lua).
--
--   * visible buffer: created by `M.open`, read-only. Its content
--     is a byte-substituted projection of source: each top-level
--     folded subtree's body bytes are replaced with a `  ...\n`
--     marker. Headlines stay in source order; navigation in the
--     visible buffer maps back to source byte offsets via the
--     projection's line index.
--
--   * fold_state: per-handle table keyed by an entry's source
--     byte_start. fold-subtree on a headline toggles its entry.
--     Fold state survives source edits because the dirty intercept
--     shifts every fold key by the same delta the parser uses on
--     the corresponding entry's byte_start. Folds whose headline
--     byte itself was deleted (delete or replace ranges that
--     include the entry's `*` byte) are dropped, matching what the
--     parser does to the entry itself. If a fold's keyed byte ends
--     up not corresponding to any entry (e.g., user manually broke
--     the headline by inserting non-newline text), the fold is
--     silently ignored on next repaint --- view.render only honors
--     keys that match a current entry's byte_start.
--
--   * repaint cadence: lazy. A source edit marks the projection
--     dirty in the source intercept; the next visible-buffer
--     operation (fold toggle, navigation command) repaints first,
--     then runs. This avoids paying render cost on every keystroke
--     when no view operation follows.
--
--   * intercept on visible: rejects every user edit. Package paints
--     bypass via the painting flag with a pcall guard, the
--     established CC-1 pattern (this is outline-class added to
--     CC-1's observed-in list at M8.10/M8.11).
--
-- Public surface:
--
--   local outline = require("pmacs-outline")
--   local handle = outline.open(source_buf)   -- creates visible projection
--                                                 paired with source_buf;
--                                                 returns handle with
--                                                 .source / .visible /
--                                                 .parser_handle
--   outline.close(handle)                     -- removes intercepts,
--                                                 drops the visible buffer
--   outline.query(source_buf, predicate)      -- public structure query;
--                                                 also installed as
--                                                 pmacs.outline.query
--
--   M-x pmacs-outline.next-headline           -- in visible buffer
--   M-x pmacs-outline.parent-headline
--   M-x pmacs-outline.fold-subtree            -- toggle fold of entry
--                                                under cursor
--
-- Default keybindings on the *visible* buffer (read-only, so these
-- don't shadow text input):
--
--   n      next-headline
--   p      parent-headline
--   Tab    fold-subtree
--
-- API surface and the M8 ten-API ceiling: outline-class adds 0 new
-- `pmacs.*` (Rust-bridge) APIs --- the M8.12 ceiling is for the Lua
-- bindings exposed from Rust, not for package-author Lua surfaces.
-- The package *does* expose package-level Lua surface: `M.open` and
-- `M.close` on the entry module, plus the three submodule exports
-- (parser / view / nav) which the package loader requires in the
-- manifest's `exports` list so in-package requires resolve. The
-- function-level contracts on those submodules are package-internal
-- by convention; external consumers reaching into them are using a
-- private surface (the `reach-around-require` audit-lint rule and
-- the SP-3 v0.2-prerequisite address this for v0.2 enforcement).
-- This is the same disposition the M8.4 and M8.8 audits applied to
-- dired-class and magit-class respectively.

local parser = require("pmacs-outline.parser")
local view = require("pmacs-outline.view")
local nav = require("pmacs-outline.nav")
local aggregate = require("pmacs-outline.aggregate")

local M = {}

-- ---------------------------------------------------------------------------
-- Per-handle state
-- ---------------------------------------------------------------------------

local handles = {}

local function find_handle_by(field, buf)
  local live, found = {}, nil
  for _, h in ipairs(handles) do
    local src_ok, src_valid = pcall(h.source.is_valid, h.source)
    local vis_ok, vis_valid = pcall(h.visible.is_valid, h.visible)
    if src_ok and src_valid and vis_ok and vis_valid then
      live[#live + 1] = h
      if h[field] == buf then found = h end
    else
      M.close(h)
    end
  end
  handles = live
  return found
end

local function active_handle()
  return find_handle_by("visible", pmacs.window.buffer())
end

-- ---------------------------------------------------------------------------
-- Painting
-- ---------------------------------------------------------------------------
--
-- The visible buffer is read-only via intercept; package paints
-- bypass via the painting flag. CC-1 pattern.

local function paint(handle, text)
  handle.painting = true
  local ok, err = pcall(function()
    handle.visible:replace(0, handle.visible:len(), text)
  end)
  handle.painting = false
  if not ok then error(err) end
end

local function make_readonly_intercept(handle)
  return function(_op)
    if handle.painting then return nil end
    error("pmacs-outline: visible projection is read-only; edit the " ..
          "source buffer instead.")
  end
end

-- ---------------------------------------------------------------------------
-- Repaint
-- ---------------------------------------------------------------------------
--
-- Synchronous repaint: read source text, query parser entries
-- (which triggers any pending lazy reparse), call view.render with
-- current fold_state, paint the visible buffer.
--
-- Repaints set handle.dirty = false on completion. Source edits
-- set handle.dirty = true via the dirty intercept; lazy callers
-- (the navigation commands) check handle.dirty and repaint first.

local function repaint(handle)
  local src = handle.source
  local source_text = src:slice(0, src:len())
  local entries = parser.entries(handle.parser_handle)
  local proj = view.render(source_text, entries, handle.fold_state)
  handle.projection = proj
  paint(handle, proj.text)
  handle.dirty = false
end

local function repaint_if_dirty(handle)
  if handle.dirty then repaint(handle) end
end

M.__pmacs_outline_test_repaint = repaint            -- for the timing test
M.__pmacs_outline_test_repaint_if_dirty = repaint_if_dirty

-- Shift fold-state keys to track entries' byte_start movements
-- through an edit. fold_state is keyed by the source byte_start of
-- the folded entry; when the parser shifts an entry through an edit
-- (insert before / delete before / replace before), the fold key
-- has to track the same shift or the fold is silently lost on the
-- next repaint (Pass-3 finding 3).
--
-- Rules per op:
--   insert (pos, n):            keys >= pos shift +n.
--   delete  (s, e):             keys < s unchanged; keys in [s, e]
--                                 dropped (the entry's headline byte
--                                 was deleted, so the entry itself
--                                 is gone from the cache); keys > e
--                                 shift -n where n = e-s.
--   replace (s, e, n_new):      same shape as delete with delta =
--                                 n_new - (e-s); keys in [s, e]
--                                 dropped.
--
-- Edge case for delete at exactly k == s: the byte AT the entry's
-- byte_start was deleted (i.e., the leading `*` of the headline).
-- That entry is gone; drop the fold.
local function shift_fold_keys(handle, op)
  local fs = handle.fold_state
  if next(fs) == nil then return end
  local out = {}
  if op.kind == "insert" then
    local pos, n = op.pos, op.bytes_len
    for k, v in pairs(fs) do
      if k >= pos then out[k + n] = v else out[k] = v end
    end
  elseif op.kind == "delete" then
    local s, e = op.start, op["end"]
    local n = e - s
    for k, v in pairs(fs) do
      if k < s then
        out[k] = v
      elseif k >= e then
        out[k - n] = v
      end
      -- else: k in [s, e), the fold's headline byte was deleted; drop.
    end
  elseif op.kind == "replace" then
    local s, e = op.start, op["end"]
    local delta = op.bytes_len - (e - s)
    for k, v in pairs(fs) do
      if k < s then
        out[k] = v
      elseif k >= e then
        out[k + delta] = v
      end
    end
  end
  handle.fold_state = out
end

-- The dirty-marker intercept on the source buffer. Runs *in addition
-- to* the parser's own intercept (intercept chain). Both return nil;
-- both observe the edit. The parser intercept updates cache; this
-- one marks the visible projection stale and tracks fold-key shifts.
local function make_dirty_intercept(handle)
  return function(op)
    handle.dirty = true
    shift_fold_keys(handle, op)
    return nil
  end
end

-- ---------------------------------------------------------------------------
-- Public-to-package: open / close
-- ---------------------------------------------------------------------------

function M.open(source_buf)
  local existing = find_handle_by("source", source_buf)
  if existing then return existing end

  local visible = pmacs.buffer.create("*outline:" .. source_buf:name() .. "*")
  local parser_handle = parser.attach(source_buf)

  local handle = {
    source = source_buf,
    visible = visible,
    parser_handle = parser_handle,
    fold_state = {},
    painting = false,
    dirty = true,
    projection = nil,
  }
  handles[#handles + 1] = handle

  handle.dirty_intercept = pmacs.buffer.add_intercept(
    source_buf, make_dirty_intercept(handle))
  handle.readonly_intercept = pmacs.buffer.add_intercept(
    visible, make_readonly_intercept(handle))

  -- Initial render: synchronous so the visible buffer has content
  -- before `open` returns. Bullet 1's 100ms budget covers this path.
  repaint(handle)

  pmacs.keymap.bind {
    scope = "buffer", buffer = visible, sequence = "n",
    command = "pmacs-outline.next-headline",
  }
  pmacs.keymap.bind {
    scope = "buffer", buffer = visible, sequence = "p",
    command = "pmacs-outline.parent-headline",
  }
  pmacs.keymap.bind {
    scope = "buffer", buffer = visible, sequence = "Tab",
    command = "pmacs-outline.fold-subtree",
  }

  pmacs.window.switch_buffer(visible)
  return handle
end

function M.close(handle)
  if handle.dirty_intercept then
    pcall(pmacs.buffer.remove_intercept, handle.dirty_intercept)
    handle.dirty_intercept = nil
  end
  if handle.readonly_intercept then
    pcall(pmacs.buffer.remove_intercept, handle.readonly_intercept)
    handle.readonly_intercept = nil
  end
  if handle.parser_handle then
    parser.detach(handle.parser_handle)
    handle.parser_handle = nil
  end
  if handle.visible and pmacs.buffer.kill then
    pcall(pmacs.buffer.kill, handle.visible)
  end
  -- Filter out of `handles`.
  local kept = {}
  for _, x in ipairs(handles) do
    if x ~= handle then kept[#kept + 1] = x end
  end
  handles = kept
end

-- Toggle fold for the entry at the given source byte offset. Used
-- by the fold-subtree command and exposed for tests.
function M.toggle_fold(handle, source_byte)
  local entry = parser.entry_at(handle.parser_handle, source_byte)
  if not entry then return end
  if handle.fold_state[entry.byte_start] then
    handle.fold_state[entry.byte_start] = nil
  else
    handle.fold_state[entry.byte_start] = true
  end
  repaint(handle)
end

function M.query(source_buf, predicate)
  if type(predicate) ~= "function" then
    error("pmacs-outline.query: predicate must be a function")
  end

  local h = find_handle_by("source", source_buf)
  if h then
    return parser.query(h.parser_handle, predicate)
  end

  local ph = parser.attach(source_buf)
  local ok, result = pcall(function()
    return parser.query(ph, predicate)
  end)
  parser.detach(ph)
  if not ok then error(result) end
  return result
end

pmacs.outline = pmacs.outline or {}
pmacs.outline.query = M.query

-- ---------------------------------------------------------------------------
-- Commands
-- ---------------------------------------------------------------------------

local OWNED_COMMANDS = {}

local function define_owned(spec)
  pmacs.command.define(spec)
  OWNED_COMMANDS[#OWNED_COMMANDS + 1] = spec.name
end

define_owned {
  name = "pmacs-outline.next-headline",
  description = "Move cursor to the next headline.",
  fn = function()
    local h = active_handle()
    if not h then return end
    repaint_if_dirty(h)
    nav.next_headline(h, parser, view)
  end,
}

define_owned {
  name = "pmacs-outline.parent-headline",
  description = "Move cursor to the parent of the current headline.",
  fn = function()
    local h = active_handle()
    if not h then return end
    repaint_if_dirty(h)
    nav.parent_headline(h, parser, view)
  end,
}

define_owned {
  name = "pmacs-outline.fold-subtree",
  description = "Toggle fold of the subtree under cursor.",
  fn = function()
    local h = active_handle()
    if not h then return end
    repaint_if_dirty(h)
    local cur_visible_line = pmacs.editor.cursor_line()
    local source_byte = view.source_byte_at_visible_line(
      h.projection, cur_visible_line)
    M.toggle_fold(h, source_byte)
  end,
}

-- ---------------------------------------------------------------------------
-- Cleanup on unload
-- ---------------------------------------------------------------------------

pmacs.packages.on_unload(function()
  -- Pass-6 finding 2: close live aggregate handles before tearing
  -- down outline view handles. Aggregates hold source-listener
  -- intercepts, parser refcounts, and aggregate buffers that need
  -- the same dispose discipline. Without this, reloading the
  -- package leaves stale intercepts on the user's source buffers
  -- pointing at the old, now-discarded module's closures.
  aggregate.close_all_handles()

  -- Iterate a snapshot since close() mutates `handles`.
  local snapshot = {}
  for _, h in ipairs(handles) do snapshot[#snapshot + 1] = h end
  for _, h in ipairs(snapshot) do M.close(h) end
  handles = {}
  for _, name in ipairs(OWNED_COMMANDS) do
    pmacs.command.unregister(name)
  end
  OWNED_COMMANDS = {}
  if pmacs.outline and pmacs.outline.query == M.query then
    pmacs.outline.query = nil
  end
end)

-- ---------------------------------------------------------------------------
-- Test seam
-- ---------------------------------------------------------------------------

-- ---------------------------------------------------------------------------
-- Aggregate buffer (T M8.10)
-- ---------------------------------------------------------------------------

M.aggregate = aggregate.aggregate
M.aggregate_close = aggregate.aggregate_close

M.__pmacs_outline_test_seam_DO_NOT_USE = {
  parser = parser,
  view = view,
  nav = nav,
  aggregate = aggregate,
  active_handle = active_handle,
  find_handle_by = find_handle_by,
  paint = paint,
  repaint = repaint,
}

return M
