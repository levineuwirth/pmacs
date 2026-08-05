-- listview.lua --- reusable read-only list panels (Arc 1b, Q#P1).
--
-- Generalizes the *buffer-list* idiom (builtin/commands/default.lua)
-- into `pmacs.listview.open{...}`: a persistent named buffer,
-- wholesale re-render, buffer-local RET/n/p/g/q keymap, a
-- line->item map, previous-buffer capture + `q` restore, and the two
-- disciplines the hand-rolled original lacks --- a read-only
-- intercept (Q#P3) and the Q#P6 round-trip-input mark, so a
-- semantic frontend's RET dispatches into the visit binding instead
-- of optimistically inserting a newline.
--
-- Generated-buffer immutability (Q#GB1, docs/generated-buffer-immutability-framing.md):
-- a panel's rope is genuinely read-only, and `render` is its owner's one
-- authorized door through the lock. The intercept alone protected the
-- edit path and left the history path open, so `C-/` emptied a panel.
-- Ownership is the `panels` table, never a name match (Q#GB13/Q#GB18).
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

-- panels: array of
--   { requested_name, buffer, prev, header, line_to_item, on_visit, on_refresh }
--
-- A LIST scanned by identity, not a name-keyed map (Q#GB18). `panels`
-- used to be written under the name the CALLER asked for and read back
-- under the buffer's ACTUAL name; those are the same string only while
-- `ensure_panel` adopts whatever buffer already carries the name. Once
-- ownership disambiguates a collision to `*references*<2>` (Q#GB13), a
-- name-keyed lookup can never find its own record, and every consumer
-- below fails: `RET`, `g` and `q` fail closed and silently, while
-- `open`'s capture guard fails OPEN and captures a panel as its own `q`
-- target --- the chained-panel loop its comment says it prevents.
--
-- Keyed by linear scan over `BufferIdLua.__eq` rather than by table key
-- for the same reason dired's `handles` is (dired.lua:120-140): two
-- BufferIdLua values for the same buffer are distinct userdata, so a
-- `panels[buf]` lookup would miss. `compile.lua`'s `slot_for_buffer`
-- is the third instance of this shape; listview adopts it rather than
-- inventing a fourth.
--
-- Dead panels are compacted out on every scan. A map held at most one
-- entry per name and self-limited; a list does not, so killing and
-- reopening `*references*` ten times would otherwise leave nine dead
-- records for every scan to walk.
local panels = {}

-- How far the `<2>`, `<3>`, ... disambiguation walks before giving up.
-- dired.lua:474's constant, same value, same give-up-rather-than-adopt
-- rule.
local NAME_VARIANT_LIMIT = 99

local function find_buffer_by_name(name)
  for _, id in ipairs(pmacs.buffer.list()) do
    local ok, d = pcall(pmacs.describe.buffer, id)
    if ok and d and d.name == name then return id end
  end
  return nil
end

local function live_panels()
  local live = {}
  for _, p in ipairs(panels) do
    local ok, valid = pcall(p.buffer.is_valid, p.buffer)
    if ok and valid then live[#live + 1] = p end
  end
  panels = live
  return live
end

-- The record for the panel `spec.name` asked for. Stable across
-- disambiguation: a repeated `listview.open{ name = "*references*" }`
-- must reach the same panel even when its buffer is called
-- `*references*<2>`.
local function panel_for_requested_name(name)
  for _, p in ipairs(live_panels()) do
    if p.requested_name == name then return p end
  end
  return nil
end

-- The record that owns `buf`, or nil. This is the identity question
-- every command below actually asks.
local function panel_for_buffer(buf)
  if buf == nil then return nil end
  for _, p in ipairs(live_panels()) do
    if p.buffer == buf then return p end
  end
  return nil
end

-- The panel record whose buffer the active window shows, or nil.
local function active_panel()
  return panel_for_buffer(pmacs.window.buffer())
end

-- Wholesale re-render: header + one line per row, rebuilding the
-- line->item map (data lines are 1-based; the header is line 0).
--
-- One `set_generated_contents` (the owner-authorized write) rather than
-- a delete-all + insert-all pair through `bypass_intercept`. The
-- intercept guarded the edit path and left the HISTORY path open, so a
-- bare `C-/` --- listview rebinds no undo chord --- emptied the panel;
-- `M-x buffer.undo` did too, and no rebinding can remove that. The
-- primitive lifts the rope lock, writes, discards the history and
-- re-asserts the lock, all inside one registry borrow.
-- Tree support (docs/tree-primitive-framing.md, Q#TR1-TR4).
--
-- A row MAY carry `depth` (0-based, structural) and `id` (opaque,
-- consumer-supplied, compared by equality). Both optional: a row
-- without them behaves exactly as before, which is what keeps the
-- three flat consumers byte-identical.
--
-- `text` stays CONSUMER-RENDERED (Q#TR4). The primitive owns structure,
-- not presentation -- collapse only ever HIDES rows and never changes a
-- surviving row's depth, so pre-rendered indentation remains correct
-- and the primitive never has to re-format anything.
--
-- Descendants are a CONTIGUOUS RUN of following rows with greater
-- depth. That holds because consumers emit parents before children in
-- document order (the LSP outline's `Symbol` ordering guarantees it);
-- a consumer that emits depth out of order gets nonsense, which is why
-- `has_children` reads only the NEXT row rather than scanning.
local function has_children(rows, i)
  local d = rows[i].depth
  if not d then return false end
  local nxt = rows[i + 1]
  return nxt ~= nil and (nxt.depth or 0) > d
end

-- Is `rows[i]` hidden because some ANCESTOR is collapsed?
--
-- Walks backwards to shallower rows, which is the ancestor chain under
-- the contiguous-run invariant above. Stops at depth 0: a root has no
-- ancestor to hide it.
local function hidden_by_ancestor(p, rows, i)
  local d = rows[i].depth
  if not d or d == 0 then return false end
  local want = d - 1
  for j = i - 1, 1, -1 do
    local dj = rows[j].depth or 0
    if dj <= want then
      if rows[j].id ~= nil and p.collapsed[rows[j].id] then return true end
      want = dj - 1
      if want < 0 then return false end
    end
  end
  return false
end

local function render(p, rows)
  local lines = { p.header }
  p.line_to_item = {}
  p.line_to_row = {}
  for i, row in ipairs(rows) do
    if not hidden_by_ancestor(p, rows, i) then
      lines[#lines + 1] = row.text
      p.line_to_item[#lines - 1] = row.item
      p.line_to_row[#lines - 1] = row
    end
  end
  pmacs.buffer.set_generated_contents(p.buffer, table.concat(lines, "\n"))
end

-- The data line currently showing `id`, or nil. Selection is re-seated
-- BY ID rather than by line (Q#TR3): a collapse or expand inserts or
-- removes rows above the cursor, so a line-keyed restore lands on an
-- unrelated node.
local function line_of_id(p, id)
  if id == nil then return nil end
  for line, row in pairs(p.line_to_row) do
    if row.id ~= nil and row.id == id then return line end
  end
  return nil
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
  bind("TAB", "listview.toggle")
  bind("g", "listview.refresh")
  bind("q", "listview.quit")
end

-- Build the persistent panel record for `name`. A user-killed panel
-- buffer is compacted out by `live_panels`, so the next `open` builds a
-- fresh record rather than resurrecting a dead one.
--
-- Q#GB13: found-by-name is NOT adoption. `pmacs.buffer.create` takes any
-- caller-chosen name, so a foreign buffer may already be called
-- `*references*`; this used to adopt it, clobber the user's bytes, and
-- install an erroring intercept whose handle it discarded --- leaving
-- the user's buffer permanently un-editable. Rendering through
-- `set_generated_contents` would additionally lock its rope and clear
-- the history, removing the `M-x buffer.undo` that is currently the only
-- way back. So ownership is "this buffer is in `panels`", a name
-- collision disambiguates `<2>`..`<99>`, and exhausting the limit raises
-- rather than adopting --- the rule terminal.lua:300-305 states and
-- dired.lua:476-504 already implements.
local function ensure_panel(name)
  local p = panel_for_requested_name(name)
  if p then return p end

  local actual = name
  if find_buffer_by_name(actual) then
    local unique = nil
    for i = 2, NAME_VARIANT_LIMIT do
      local candidate = string.format("%s<%d>", name, i)
      if find_buffer_by_name(candidate) == nil then
        unique = candidate
        break
      end
    end
    if unique == nil then
      error(string.format("listview: %s is taken and no free variant remains", name))
    end
    actual = unique
  end

  local buf = pmacs.buffer.create(actual)
  p = { requested_name = name, buffer = buf, line_to_item = {},
        line_to_row = {}, collapsed = {}, rows = {} }
  panels[#panels + 1] = p
  -- Read-only (Q#P3): every non-bypass edit is rejected, with a NAMED
  -- error. Kept beside the rope lock, not replaced by it: the layering
  -- at terminal.lua:351-366 --- the rope lock protects the daemon copy,
  -- this and the round-trip mark protect a semantic frontend's own
  -- mirror, and neither substitutes for the other. The intercept lives
  -- as long as the buffer; no teardown (the buffer-list precedent for
  -- its keymap).
  pmacs.buffer.add_intercept(buf, function()
    error(actual .. " is read-only")
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
  if active and not panel_for_buffer(active) then
    p.prev = active
  end
  -- Keep the row array: collapse re-renders from it WITHOUT calling the
  -- consumer, which is what lets a panel with no `on_refresh` still
  -- expand and collapse (the outline has none -- framing §1.5a).
  p.rows = spec.rows or {}
  p.collapsed = {}
  render(p, p.rows)
  -- Bottom-panel arc (Q#BP11b): the placement opt-in. `seat_cursor` and
  -- `listview.refresh` are active-window-only, so an interactive panel
  -- MUST take `select = true` or it would silently seat the wrong
  -- window. In Stages 1-2 omitting `display` keeps today's raw switch;
  -- Stage 3 flips the default. An unknown value errors before anything
  -- is displayed.
  -- Q#S3-1: the vocabulary, the error and the default policy are one
  -- rule (`window._resolve_display`), not a copy per adopter. The
  -- default is passed in because the adopters do not share one.
  -- Stage 3 (Q#BP12): omission resolves to the PANEL. `select = true`
  -- below is a correctness requirement, not a preference — `seat_cursor`
  -- and `listview.refresh` drive `pmacs.editor.move_down()`, which acts
  -- on the ACTIVE window, so an unselected panel would seat the cursor
  -- in the user's document.
  local display = pmacs.window._resolve_display("listview.open", spec.display, "panel")
  if display == "panel" then
    pmacs.window.display(p.buffer, { side = "bottom", select = true })
  else
    pmacs.window.switch_buffer(p.buffer)
  end
  seat_cursor(p, 1)
end

pmacs.command.define {
  name = "listview.visit",
  description = "Visit the list-panel item under the cursor.",
  fn = function()
    local p = active_panel()
    if not p then return end
    local item = p.line_to_item[pmacs.editor.cursor_line()]
    if item ~= nil and p.on_visit then p.on_visit(item) end
  end,
}

pmacs.command.define {
  name = "listview.refresh",
  description = "Re-run the list panel's data source and re-render.",
  fn = function()
    local p = active_panel()
    if not (p and p.on_refresh) then return end
    local saved = pmacs.editor.cursor_line()
    -- Q#TR3: remember the NODE, not the line. A refresh that changes
    -- the row set moves every line; the id survives it.
    local saved_row = p.line_to_row[saved]
    local saved_id = saved_row and saved_row.id
    local rows = p.on_refresh() or {}
    p.rows = rows
    render(p, rows)
    -- `set_generated_contents` has already refreshed this window's
    -- TextView. Re-seat through the editor primitives instead of
    -- switching to the buffer it already shows: that redundant switch
    -- rebuilt the TextView and hid a missing edit notification.
    pmacs.editor.clear_selection()
    pmacs.editor.set_view_top(0)
    pmacs.editor.move_to_line(0)
    seat_cursor(p, line_of_id(p, saved_id) or saved)
  end,
}

-- TAB toggles the node under the cursor. A leaf is a no-op with a
-- status, never a silent nothing -- the outline's `g` is already a
-- dead binding that responds to nothing (framing §1.3a) and this
-- primitive should not add a second one.
pmacs.command.define {
  name = "listview.toggle",
  description = "Collapse or expand the tree node under the cursor.",
  fn = function()
    local p = active_panel()
    if not p then return end
    local line = pmacs.editor.cursor_line()
    local row = p.line_to_row[line]
    if not (row and row.id ~= nil) then
      pmacs.editor.set_status("listview: no node here")
      return
    end
    -- `has_children` reads the FULL row array, not the rendered subset:
    -- a collapsed node's children are absent from `line_to_row` by
    -- construction, so asking the rendered view whether it has any
    -- would answer "no" for every collapsed node and make expanding
    -- impossible.
    local idx
    for i, r in ipairs(p.rows) do
      if r.id ~= nil and r.id == row.id then idx = i break end
    end
    if not (idx and has_children(p.rows, idx)) then
      pmacs.editor.set_status("listview: no children")
      return
    end
    p.collapsed[row.id] = not p.collapsed[row.id] or nil
    render(p, p.rows)
    pmacs.editor.clear_selection()
    pmacs.editor.set_view_top(0)
    pmacs.editor.move_to_line(0)
    seat_cursor(p, line_of_id(p, row.id) or line)
  end,
}

pmacs.command.define {
  name = "listview.quit",
  description = "Leave the list panel, restoring the previous buffer.",
  fn = function()
    local p = active_panel()
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
