-- builtin/menus/default.lua --- default right-click context menu (Q#CM2/Q#CM3).
--
-- Items are registered with `pmacs.menu.item` (Rust registry). Each
-- names a command to invoke and declares visibility via a coarse
-- `context` tag (sugar) or a full `predicate`, evaluated against a
-- context table when the menu opens. `group`/`order` drive layout;
-- separators fall between groups.
--
-- `pmacs.menu.build()` (called from Rust at right-click) does the
-- resolution: filter visible items, group/sort, and return the rows.

local ed = pmacs.editor

-- The active buffer's live LSP attachment record, or nil --- guarded so
-- the menu never errors when LSP isn't configured/loaded, and never
-- triggers an attach just by opening.
local function lsp_attachment()
  if pmacs.lsp == nil or pmacs.lsp.active_attachment == nil then
    return nil
  end
  local ok, rec = pcall(pmacs.lsp.active_attachment)
  if ok then return rec end
  return nil
end

-- Whether the 0-based (line, col) falls within diagnostic `d`'s range.
local function diag_contains(d, line, col)
  if line < d.start_line or line > d.end_line then return false end
  if line == d.start_line and col < d.start_col then return false end
  if line == d.end_line and col > d.end_col then return false end
  return true
end

-- Evaluate a coarse `context` tag against the live context table
-- (Q#CM3). Sugar for a predicate; `symbol` needs a word-under-point and
-- an attached server, `diagnostic` needs a published diagnostic
-- spanning the cursor.
function pmacs.menu._context_eval(tag, cx)
  if tag == "always" then
    return true
  elseif tag == "selection" then
    return cx.has_selection
  elseif tag == "symbol" then
    return cx.word ~= nil and cx.attachment ~= nil
  elseif tag == "diagnostic" then
    if cx.attachment == nil then return false end
    for _, d in ipairs(pmacs.diag.list(cx.attachment.uri)) do
      if diag_contains(d, cx.line, cx.col) then return true end
    end
    return false
  end
  return false
end

-- Whether `it` is visible in context `cx`. A failing predicate hides
-- the item rather than aborting the whole menu.
local function item_visible(it, cx)
  if it.predicate ~= nil then
    local ok, vis = pcall(it.predicate, cx)
    return ok and vis and true or false
  elseif it.context ~= nil then
    return pmacs.menu._context_eval(it.context, cx) and true or false
  end
  return true
end

-- Build the resolved, grouped, visibility-filtered rows for an open
-- menu (Q#CM3). Returns an array where each element is either
-- `{ separator = true }` or `{ label = ..., command = ... }`.
function pmacs.menu.build()
  local cx = {
    has_selection = ed.region() ~= nil,
    word = ed.word_at_cursor(),
    line = ed.cursor_line(),
    col = ed.cursor_col(),
    attachment = lsp_attachment(),
  }

  -- Filter to visible items, tagging insertion order for a stable sort.
  local visible = {}
  for i, it in ipairs(pmacs.menu._raw()) do
    if item_visible(it, cx) then
      it.__i = i
      visible[#visible + 1] = it
    end
  end

  -- Group order = first appearance in the registry; within a group,
  -- sort by `order`, then by insertion for ties.
  local gidx, next_g = {}, 1
  for _, it in ipairs(visible) do
    local g = it.group or ""
    if gidx[g] == nil then
      gidx[g] = next_g
      next_g = next_g + 1
    end
  end
  table.sort(visible, function(a, b)
    local ga, gb = gidx[a.group or ""], gidx[b.group or ""]
    if ga ~= gb then return ga < gb end
    local oa, ob = a.order or 0, b.order or 0
    if oa ~= ob then return oa < ob end
    return a.__i < b.__i
  end)

  -- Emit rows, inserting a separator between distinct groups.
  local rows, last_group = {}, nil
  for _, it in ipairs(visible) do
    local g = it.group or ""
    if last_group ~= nil and g ~= last_group then
      rows[#rows + 1] = { separator = true }
    end
    rows[#rows + 1] = { label = it.label, command = it.command }
    last_group = g
  end
  return rows
end

-- Default items. The edit group adapts to the selection; the symbol
-- group appears on an identifier with a server attached; the diagnostic
-- group appears when a diagnostic spans the cursor; history is always
-- available. Group order follows registration order.
pmacs.menu.item { id = "edit.cut",        label = "Cut",        command = "edit.cut",        context = "selection", group = "edit",    order = 10 }
pmacs.menu.item { id = "edit.copy",       label = "Copy",       command = "edit.copy",       context = "selection", group = "edit",    order = 20 }
pmacs.menu.item { id = "edit.paste",      label = "Paste",      command = "edit.paste",      context = "always",    group = "edit",    order = 30 }
pmacs.menu.item { id = "edit.select-all", label = "Select All", command = "edit.select-all", context = "always",    group = "edit",    order = 40 }

-- Symbol group (LSP). Shown when the cursor is on an identifier and a
-- language server is attached. Each invokes the existing async command,
-- which acts at the cursor (the right-click anchored it there).
pmacs.menu.item { id = "lsp.go-to-definition", label = "Go to Definition", command = "lsp.go-to-definition", context = "symbol", group = "symbol", order = 10 }
pmacs.menu.item { id = "lsp.find-references",   label = "Find References",   command = "lsp.find-references",   context = "symbol", group = "symbol", order = 20 }
pmacs.menu.item { id = "lsp.rename",            label = "Rename",            command = "lsp.rename",            context = "symbol", group = "symbol", order = 30 }
pmacs.menu.item { id = "lsp.hover",             label = "Hover",             command = "lsp.hover",             context = "symbol", group = "symbol", order = 40 }

-- Diagnostic group (LSP). Shown when a diagnostic spans the cursor.
-- "Quick Fix" runs code actions at the point (Q#CM10 defers streaming
-- the individual fix titles into the menu).
pmacs.menu.item { id = "lsp.quick-fix", label = "Quick Fix", command = "lsp.code-actions", context = "diagnostic", group = "diagnostic", order = 10 }

pmacs.menu.item { id = "buffer.undo",     label = "Undo",       command = "buffer.undo",     context = "always",    group = "history", order = 10 }
pmacs.menu.item { id = "buffer.redo",     label = "Redo",       command = "buffer.redo",     context = "always",    group = "history", order = 20 }
