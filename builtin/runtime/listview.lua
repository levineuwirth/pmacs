-- listview.lua --- reusable read-only list panels (Arc 1b, Q#P1).
--
-- Generalizes the *buffer-list* idiom (builtin/commands/default.lua)
-- into `pmacs.listview.open{...}`: a persistent named buffer,
-- wholesale re-render, buffer-local RET/n/p/g/q keymap, a
-- line->item map, previous-buffer capture + `q` restore, and the two
-- disciplines the hand-rolled original lacks --- a read-only
-- intercept (Q#P3; the panel's own renders write with
-- bypass_intercept) and the Q#P6 round-trip-input mark, so a
-- semantic frontend's RET dispatches into the visit binding instead
-- of optimistically inserting a newline.
--
-- Panels are buffers, so both frontends render them with zero
-- protocol change (Q#P2: switch-in-place; the GPU cannot show
-- splits). Framing: docs/lsp-panels-framing.md.
--
--   pmacs.listview.open {
--     name = "*references*",
--     header = "12 references   RET visit  n/p move  g refresh  q quit",
--     rows = { { text = "src/foo.rs:12:4", item = <any> }, ... },
--     on_visit = function(item) ... end,      -- RET/SPC (optional)
--     on_refresh = function() return rows end, -- g (optional)
--   }

pmacs.listview = pmacs.listview or {}

-- name -> { buffer, prev, header, line_to_item, on_visit, on_refresh }
local panels = {}

local function find_buffer_by_name(name)
  for _, id in ipairs(pmacs.buffer.list()) do
    local ok, d = pcall(pmacs.describe.buffer, id)
    if ok and d and d.name == name then return id end
  end
  return nil
end

-- The panel record whose buffer the active window shows, or nil.
local function panel_for_current_buffer()
  local buf = pmacs.window.buffer()
  if not buf then return nil end
  local ok, d = pcall(pmacs.describe.buffer, buf)
  if not (ok and d) then return nil end
  return panels[d.name]
end

-- Wholesale re-render: header + one line per row, rebuilding the
-- line->item map (data lines are 1-based; the header is line 0).
-- Panel writes bypass the read-only intercept.
local function render(p, rows)
  local lines = { p.header }
  p.line_to_item = {}
  for _, row in ipairs(rows) do
    lines[#lines + 1] = row.text
    p.line_to_item[#lines - 1] = row.item
  end
  local body = table.concat(lines, "\n")
  local buf = p.buffer
  local len = buf:len()
  if len > 0 then buf:delete(0, len, { bypass_intercept = true }) end
  if #body > 0 then buf:insert(0, body, { bypass_intercept = true }) end
end

-- Re-seat the cursor on data line `line` (1-based, clamped).
-- `switch_active_buffer` zeroes the window cursor, so a fresh switch
-- puts us on the header; walk down from there.
local function seat_cursor(p, line)
  local count = #p.line_to_item
  if count == 0 then return end
  local target = math.max(1, math.min(line or 1, count))
  for _ = 1, target do
    pmacs.editor.move_down()
  end
end

local function bind_local_keymap(buf)
  local function bind(seq, command)
    pmacs.keymap.bind { scope = "buffer", buffer = buf, sequence = seq, command = command }
  end
  bind("RET", "listview.visit")
  bind("SPC", "listview.visit")
  bind("n", "cursor.down")
  bind("<down>", "cursor.down")
  bind("p", "cursor.up")
  bind("<up>", "cursor.up")
  bind("g", "listview.refresh")
  bind("q", "listview.quit")
end

-- Build (or adopt) the persistent panel record for `name`. Handles a
-- user-killed panel buffer by recreating it.
local function ensure_panel(name)
  local p = panels[name]
  if p and p.buffer:is_valid() then return p end
  local buf = find_buffer_by_name(name) or pmacs.buffer.create(name)
  p = { buffer = buf, line_to_item = {} }
  panels[name] = p
  -- Read-only (Q#P3): every non-bypass edit is rejected. The
  -- intercept lives as long as the buffer; no teardown (the
  -- buffer-list precedent for its keymap).
  pmacs.buffer.add_intercept(buf, function()
    error(name .. " is read-only")
  end)
  -- Q#P6: semantic frontends must round-trip keys while this panel
  -- is focused (RET = visit, not an optimistic newline).
  pmacs.buffer.set_round_trip_input(buf, true)
  bind_local_keymap(buf)
  return p
end

function pmacs.listview.open(spec)
  assert(type(spec) == "table" and type(spec.name) == "string",
    "listview.open: spec.name (string) required")
  local p = ensure_panel(spec.name)
  p.header = spec.header or spec.name
  p.on_visit = spec.on_visit
  p.on_refresh = spec.on_refresh
  -- Remember where to return on `q` --- but never another panel
  -- (chained panels would trap `q` in a loop; restore targets the
  -- last real buffer).
  local active = pmacs.window.buffer()
  if active and not panel_for_current_buffer() then
    p.prev = active
  end
  render(p, spec.rows or {})
  -- Bottom-panel arc (Q#BP11b): the placement opt-in. `seat_cursor` and
  -- `listview.refresh` are active-window-only, so an interactive panel
  -- MUST take `select = true` or it would silently seat the wrong
  -- window. In Stages 1-2 omitting `display` keeps today's raw switch;
  -- Stage 3 flips the default. An unknown value errors before anything
  -- is displayed.
  local display = spec.display
  if display ~= nil and display ~= "current" and display ~= "panel" then
    error(string.format(
      "listview.open: unknown display %q (expected \"current\" or \"panel\")",
      tostring(display)))
  end
  if display == "panel" then
    p.side = true
    pmacs.window.display(p.buffer, { side = "bottom", select = true })
  else
    p.side = false
    pmacs.window.switch_buffer(p.buffer)
  end
  seat_cursor(p, 1)
end

pmacs.command.define {
  name = "listview.visit",
  description = "Visit the list-panel item under the cursor.",
  fn = function()
    local p = panel_for_current_buffer()
    if not p then return end
    local item = p.line_to_item[pmacs.editor.cursor_line()]
    if item ~= nil and p.on_visit then p.on_visit(item) end
  end,
}

pmacs.command.define {
  name = "listview.refresh",
  description = "Re-run the list panel's data source and re-render.",
  fn = function()
    local p = panel_for_current_buffer()
    if not (p and p.on_refresh) then return end
    local saved = pmacs.editor.cursor_line()
    local rows = p.on_refresh() or {}
    render(p, rows)
    -- The wholesale rewrite leaves the window cursor at a stale byte
    -- offset; re-enter the buffer to reset, then re-seat.
    pmacs.window.switch_buffer(p.buffer)
    seat_cursor(p, saved)
  end,
}

pmacs.command.define {
  name = "listview.quit",
  description = "Leave the list panel, restoring the previous buffer.",
  fn = function()
    local p = panel_for_current_buffer()
    if not p then return end
    -- Bottom-panel arc (Q#BP11b): `q` keeps its name and its
    -- user-visible behavior, delegating to `window.quit` only when the
    -- listview really is in a side window. Capability fallback (and any
    -- pre-arc placement) keeps the previous-buffer switch below.
    local params = pmacs.window.params()
    if params and params.side and params.quit_action then
      pmacs.window.quit()
      return
    end
    local target = p.prev
    if not (target and target:is_valid()) then
      target = find_buffer_by_name("*scratch*") or pmacs.buffer.create("*scratch*")
    end
    pmacs.window.switch_buffer(target)
  end,
}
