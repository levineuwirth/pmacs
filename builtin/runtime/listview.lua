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
-- Generated-buffer immutability (Q#GB1, docs/archive/framings/generated-buffer-immutability-framing.md):
-- a panel's rope is genuinely read-only, and `render` is its owner's one
-- authorized door through the lock. The intercept alone protected the
-- edit path and left the history path open, so `C-/` emptied a panel.
-- Ownership is the `panels` table, never a name match (Q#GB13/Q#GB18).
--
-- Panels are buffers, so both frontends render them with zero
-- protocol change (Q#P2: switch-in-place; the GPU cannot show
-- splits). Framing: docs/archive/framings/lsp-panels-framing.md.
--
--   pmacs.listview.open {
--     name = "*references*",
--     header = "12 references   RET visit  n/p move  g refresh  q quit",
--     rows = { { text = "src/foo.rs:12:4", item = <any> }, ... },
--     on_visit = function(item) ... end,      -- RET/SPC (optional)
--     on_refresh = function() return rows end, -- g (optional)
--     keys = { d = "git.diff-file" },          -- extra buffer-local keys
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
-- Tree support (docs/archive/framings/tree-primitive-framing.md, Q#TR1-TR4).
--
-- A row MAY carry `depth` (0-based, structural) and `id` (a STRING or
-- NUMBER, consumer-supplied, compared by value). Both optional: a row
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

-- Ids must be usable, unique table keys, and none of the three checks
-- below is fussiness about types.
--
-- SCALAR. Selection compares ids with `==`, which honours `__eq`;
-- collapse state stores them as TABLE KEYS, and Lua indexes tables by
-- raw identity, consulting no metamethod. A table id would therefore
-- satisfy one and quietly fail the other: after a refresh minted fresh
-- id tables, selection would be restored and the fold would be lost.
--
-- Equality-aware collapse lookup is the alternative, and it is worse
-- here: `hidden_by_ancestor` runs per row and would turn a linear
-- render quadratic to support a key type no consumer has wanted.
--
-- NOT NaN. `0/0` passes a `type(x) == "number"` test and then *errors*
-- at `p.collapsed[row.id]` with "table index is NaN" — the one scalar
-- Lua accepts as a number and refuses as a key. Caught here so the
-- report names the row, rather than surfacing on whichever later TAB
-- happens to reach it.
--
-- UNIQUE. Every lookup here resolves an id to the FIRST row bearing
-- it, so duplicates do not merely collide: selecting the second such
-- row toggles the first and re-seats the cursor onto it. An id that
-- does not identify a node is not an id, and the contract's word for
-- itself is identity.
--
-- All three are enforced where rows enter, so a bad id is a named
-- error at the call site instead of a lost fold or a stray jump later.
local function check_ids(rows)
  local seen = {}
  for i, row in ipairs(rows) do
    local id, k = row.id, type(row.id)
    if id ~= nil then
      if k ~= "string" and k ~= "number" then
        error(string.format(
          "listview: row %d has a %s id; ids must be a string or number "
          .. "(collapse state keys a table by identity, so a %s id would "
          .. "lose its fold across a refresh)", i, k, k))
      end
      if id ~= id then
        error(string.format(
          "listview: row %d has a NaN id; NaN is a number but not a "
          .. "usable table key, and collapse state would raise "
          .. "\"table index is NaN\" on the first fold", i))
      end
      if seen[id] then
        error(string.format(
          "listview: rows %d and %d share the id %q; ids must be unique "
          .. "(every lookup resolves to the first match, so selecting "
          .. "the later row would toggle and re-seat the earlier one)",
          seen[id], i, tostring(id)))
      end
      seen[id] = i
    end
  end
  return rows
end

local function render(p, rows)
  local lines = { p.header }
  p.visible = 0
  p.line_to_item = {}
  p.line_to_row = {}
  for i, row in ipairs(rows) do
    if not hidden_by_ancestor(p, rows, i) then
      lines[#lines + 1] = row.text
      -- SPARSE BY CONSTRUCTION: `item` is optional, and a display-only
      -- row (a grouping header in a tree, say) supplies none, so this
      -- key is simply absent for it. Nothing may take `#` of this
      -- table; `visible` below is the row count.
      p.line_to_item[#lines - 1] = row.item
      p.line_to_row[#lines - 1] = row
      p.visible = #lines - 1
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
  -- `p.visible`, NOT `#p.line_to_item`: that map is sparse whenever a
  -- row omits the optional `item`, and `#` on a sparse table is not
  -- the row count. Reading it there left a tree of display-only rows
  -- with the cursor stranded on the header, where TAB finds no node.
  local count = p.visible or 0
  if count == 0 then return end
  local target = math.max(1, math.min(line or 1, count))
  for _ = 1, target do
    pmacs.editor.move_down()
  end
end

-- The primitive's own key surface, named ONCE so the binder below and
-- the `keys` validator consult the same list. Previously this was a
-- sequence of `bind(...)` calls and the set existed nowhere as data,
-- which is why the git framing had to quote it from the source
-- (docs/archive/framings/git-integration-framing.md Q#G-7).
local FIXED_KEYS = {
  { "RET", "listview.visit" },
  { "SPC", "listview.visit" },
  { "n", "cursor.down" },
  { "<down>", "cursor.down" },
  { "p", "cursor.up" },
  { "<up>", "cursor.up" },
  { "TAB", "listview.toggle" },
  { "g", "listview.refresh" },
  { "q", "listview.quit" },
}

local function bind_local_keymap(buf)
  for _, entry in ipairs(FIXED_KEYS) do
    pmacs.keymap.bind {
      scope = "buffer", buffer = buf, sequence = entry[1], command = entry[2],
    }
  end
end

-- ---------------------------------------------------------------------
-- Consumer-supplied keys (Q#G-7)
-- ---------------------------------------------------------------------
--
-- An optional `keys = { <sequence> = <command name> }` on the open
-- spec, bound through the SAME `pmacs.keymap.bind { scope = "buffer" }`
-- path as the fixed set above. It exists because a consumer cannot
-- safely bind its own key from outside: `open` disambiguates a name
-- collision to `<2>`, so the name a consumer passed is not necessarily
-- the buffer it got, and this module is the only place the handle is
-- known. No key is intercepted anywhere — COHERENCE.md §6's shadow
-- count is unchanged by this.
--
-- INSTALL-ONCE, MATCH-ON-REOPEN. `Keymap::bind` refuses duplicates
-- (`KeymapError::DuplicateBinding`, "Refuse rather than silently
-- overwrite", src/keymap_tree.rs), and a consumer built on the async
-- completion model calls `open` again on EVERY refresh. So keys are
-- installed when the buffer is created and a later `open` for a live
-- panel does not re-bind — it COMPARES, and errors on divergence.
-- Silently keeping the old binding would hand the consumer a key that
-- does something other than what it just asked for, which is the dead-
-- or-lying-key defect this module already condemns for `g`.

-- A key sequence's whitespace-separated chord tokens. That is exactly
-- how `parse_sequence` (src/key.rs) splits one, so a prefix relation
-- computed here is the same relation the trie would find.
local function chords_of(sequence)
  local out = {}
  for token in sequence:gmatch("%S+") do out[#out + 1] = token end
  return out
end

-- True when one chord list is a STRICT prefix of the other. Either
-- direction is a conflict: `Keymap` refuses both turning a leaf into a
-- submap (`WouldExtendLeaf`) and shadowing a submap with a leaf
-- (`WouldShadowSubmap`), and a `keys` table must not be able to reach
-- either.
local function prefix_conflict(a, b)
  local short, long = a, b
  if #a > #b then short, long = b, a end
  if #short == 0 or #short == #long then return false end
  for i = 1, #short do
    if short[i] ~= long[i] then return false end
  end
  return true
end

-- Normalize `keys` into a sorted array of `{ sequence, command }`.
-- Sorted so the comparison on reopen and every error message are
-- deterministic (`pairs` order is not).
local function normalized_keys(keys)
  if keys == nil then return {} end
  if type(keys) ~= "table" then
    error(string.format(
      "listview: `keys` must be a table of sequence -> command name; got %s",
      type(keys)))
  end
  local out = {}
  for sequence, command in pairs(keys) do
    if type(sequence) ~= "string" or sequence == "" then
      error("listview: every `keys` entry must be keyed by a non-empty key sequence")
    end
    if type(command) ~= "string" or command == "" then
      error(string.format(
        "listview: `keys[%q]` must be a command NAME (a non-empty string); got %s",
        sequence, type(command)))
    end
    out[#out + 1] = { sequence = sequence, command = command }
  end
  table.sort(out, function(a, b) return a.sequence < b.sequence end)
  return out
end

-- A FIRST-PASS collision check, for a better message than the keymap's.
--
-- It compares RAW TOKENS, and that is deliberately not sufficient: the
-- key parser canonicalizes aliases before it ever reaches the trie
-- (`parse_key_code`, src/key.rs, uppercases and folds `RET`/`RETURN`/
-- `ENTER`, `SPC`/`SPACE`, `ESC`/`ESCAPE`, `BS`/`BACKSPACE`,
-- `DEL`/`DELETE`), so `keys = { RETURN = ... }` is a collision this
-- function cannot see.
--
-- **`Keymap::bind` is the authority, and `ensure_panel` tears the panel
-- down when it refuses.** That is not a fallback for a check that
-- happens to be weak --- it is the only version that cannot go stale. A
-- Lua-side canonicalizer would be a second copy of `parse_key_code`'s
-- alias table, and the day the Rust one gains a name the Lua one would
-- silently stop seeing that alias, reintroducing exactly this bug for
-- it. (There is also no way to canonicalize an arbitrary sequence from
-- Lua today: `display_sequence` is reachable only through
-- `describe.key` and `keymap.list`, which both require the sequence to
-- be BOUND already.)
--
-- So what this buys is diagnosis, not safety: a named "that is the
-- panel's own `g`" instead of a raw `DuplicateBinding`.
local function check_key_collisions(entries)
  for i, entry in ipairs(entries) do
    local mine = chords_of(entry.sequence)
    for _, fixed in ipairs(FIXED_KEYS) do
      if entry.sequence == fixed[1] then
        error(string.format(
          "listview: `keys` may not rebind %q --- it is part of the panel's "
          .. "own key surface (RET SPC n <down> p <up> TAB g q), bound to %q",
          entry.sequence, fixed[2]))
      end
      if prefix_conflict(mine, chords_of(fixed[1])) then
        error(string.format(
          "listview: `keys` entry %q conflicts with the panel's own %q --- "
          .. "one is a prefix of the other, which the keymap refuses rather "
          .. "than turning a binding into a submap",
          entry.sequence, fixed[1]))
      end
    end
    for j = i + 1, #entries do
      if prefix_conflict(mine, chords_of(entries[j].sequence)) then
        error(string.format(
          "listview: `keys` entries %q and %q conflict --- one is a prefix "
          .. "of the other", entry.sequence, entries[j].sequence))
      end
    end
  end
end

-- Bind the entries, naming which one the keymap refused.
--
-- It does NOT roll back the keys it already bound: its caller owns
-- teardown, and the caller's teardown is killing the whole buffer,
-- which takes the buffer's entire keymap scope with it
-- (`after_buffer_removed` -> `KeymapStack::remove_buffer`). Unbinding
-- here as well would be a second, weaker cleanup mechanism for the same
-- failure --- and the weaker one is what let a half-built panel survive.
local function install_keys(buf, entries)
  for _, entry in ipairs(entries) do
    local ok, err = pcall(pmacs.keymap.bind, {
      scope = "buffer", buffer = buf,
      sequence = entry.sequence, command = entry.command,
    })
    if not ok then
      error(string.format(
        "listview: cannot bind %q to %q: %s",
        entry.sequence, entry.command, tostring(err)))
    end
  end
end

local function keys_match(a, b)
  if #a ~= #b then return false end
  for i = 1, #a do
    if a[i].sequence ~= b[i].sequence or a[i].command ~= b[i].command then
      return false
    end
  end
  return true
end

local function render_keys(entries)
  if #entries == 0 then return "none" end
  local parts = {}
  for i, entry in ipairs(entries) do
    parts[i] = string.format("%s=%s", entry.sequence, entry.command)
  end
  return table.concat(parts, " ")
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
local function ensure_panel(name, key_entries)
  local p = panel_for_requested_name(name)
  if p then
    -- Match-on-reopen (Q#G-7). A live panel keeps the keys it was
    -- created with; a DIFFERENT table is a consumer asking for
    -- something it will not get, so it is an error rather than a
    -- silently ignored request.
    if not keys_match(p.keys, key_entries) then
      error(string.format(
        "listview: %s is already open with keys [%s]; this open asks for "
        .. "[%s]. Keys are installed once with the panel's buffer, so the "
        .. "second table would be silently ignored --- close the panel "
        .. "first, or pass the same keys",
        name, render_keys(p.keys), render_keys(key_entries)))
    end
    return p
  end

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
        line_to_row = {}, collapsed = {}, rows = {}, visible = 0,
        keys = key_entries }
  -- ALL-OR-NOTHING from here. Everything below mutates a buffer that
  -- does not yet belong to a panel, and `install_keys` can genuinely
  -- fail: the raw-token preflight cannot see an alias spelling of a
  -- fixed key (`RETURN` for `RET`), so `Keymap::bind` is the first thing
  -- to notice, and by then the buffer exists, carries a read-only
  -- intercept and a round-trip mark, and holds the fixed keymap.
  --
  -- Leaving it behind is worse than it sounds: it is read-only, it is in
  -- no `panels` record so nothing owns or can reach it, and the next
  -- `open` for the same name finds it by name and disambiguates itself
  -- to `<2>` --- so a rejected `keys` table silently renames the panel.
  local built, err = pcall(function()
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
    install_keys(buf, key_entries)
  end)
  if not built then
    -- `kill` is the whole teardown, not a convenience: it removes the
    -- buffer AND, through `after_buffer_removed`, prunes the buffer's
    -- keymap scope, its config locals and its folds. Unbinding key by
    -- key would leave the buffer itself --- which is the defect.
    pcall(pmacs.buffer.kill, buf)
    -- Level 0: re-raise the inner message verbatim rather than stacking
    -- this line's position onto it.
    error(err, 0)
  end
  -- Registered LAST, deliberately: a failure above must leave no record
  -- claiming keys it did not bind. Nothing above needs the panel to be
  -- in `panels` --- the intercept, the round-trip mark and the keymap
  -- all address the buffer directly.
  panels[#panels + 1] = p
  return p
end

function pmacs.listview.open(spec)
  assert(type(spec) == "table" and type(spec.name) == "string",
    "listview.open: spec.name (string) required")
  -- The cheap checks first, so the common mistakes are named before
  -- anything is created. The ones this pass cannot see --- alias
  -- spellings --- are caught by `Keymap::bind` inside `ensure_panel`,
  -- which tears the panel down rather than leaving it half-built.
  local key_entries = normalized_keys(spec.keys)
  check_key_collisions(key_entries)
  local p = ensure_panel(spec.name, key_entries)
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
  p.rows = check_ids(spec.rows or {})
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
    local rows = check_ids(p.on_refresh() or {})
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
    -- A FLAT panel must keep its pre-tree TAB behaviour exactly.
    --
    -- `bind_local_keymap` binds TAB for every listview, so this command
    -- now intercepts a key that previously fell through to the global
    -- `buffer.tab` and was refused by the Q#P3 read-only intercept.
    -- Emitting a listview status instead would be a behaviour change
    -- for the three flat consumers -- invisible to a byte-identity test,
    -- which sees the buffer and not the status line or the dispatch
    -- path. So a panel with no tree rows at all delegates.
    local is_tree = false
    for _, r in ipairs(p.rows) do
      if r.id ~= nil then is_tree = true break end
    end
    if not is_tree then
      pmacs.command.invoke("buffer.tab")
      return
    end

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
