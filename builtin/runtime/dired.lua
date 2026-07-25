-- dired.lua --- the directory view (dired arc Stage 1).
--
-- Dired is not a convenience rider on an existing file surface: until
-- Stage 0 (`C-x C-f`, #162) there was no way to open a file by path at
-- all, and browsing is the half a user reaches for when they do NOT
-- already know the path. So this is a primary surface, and the one
-- thing it may never do is refuse to render a listing --- hence the
-- per-entry-tolerant `read_dir` opt it drives (Q#DR6), the only Rust
-- this stage needed besides exposing the path normalizer.
--
-- Framing: docs/dired-framing.md (Q#DR1-DR10). Stage 1 is the view:
-- listing, navigation, sort, revert, quit. Marks and operations are
-- Stage 2; the editable wdired layer is Stage 3.
--
-- Public surface:
--
--   pmacs.dired.open(path [, opts])   -- awaits; run inside pmacs.async
--     opts.display = "current" | "panel"   (Q#BP11b, default "current")
--     opts.select_name = "<basename>"      -- seat the cursor on it
--
--   M-x dired        / C-x d      -- prompt for a directory
--   M-x dired-jump   / C-x C-j    -- dired on this file's directory
--
-- In a dired buffer (mode-scoped keys, Q#DR8):
--   RET, f   visit (directory -> descend, file -> display_file)
--   ^        parent directory
--   n / p    move by line (<down> / <up> too)
--   g        revert (re-read, preserving the cursor's entry)
--   q        quit (restore the previous buffer, or window.quit in a panel)
--   s        cycle sort mode (name -> mtime -> size)
--
-- Three structural decisions worth knowing before editing this file:
--
-- 1. ONE BUFFER PER DIRECTORY, named `*dired:<canonical path>*`
--    (Q#DR2). Navigation *opens the target's buffer*; it never mutates
--    the current one. That is Emacs behavior, and it is also the only
--    way to keep the name honest --- there is no
--    `pmacs.buffer.set_name`, so the M8.2 fixture's in-place repaint
--    leaves a buffer named after a directory it no longer shows.
--
-- 2. THE CANONICAL FORM IS THE CORE'S, not a copy of it
--    (`pmacs.path.canonicalize` is `normalize_buffer_path` itself).
--    Dired's name-dedup and `display_file`'s `find_buffer_for_path`
--    dedup have to agree; two implementations that disagree on `//tmp`
--    or a `..` at root would mint two buffers for one directory with no
--    error anywhere.
--
-- 3. EVERY LISTING IS ASYNC. `pmacs.fs.read_dir` is worker-dispatched,
--    so each command spawns a coroutine and the work after the first
--    `:await()` resumes on a later tick --- outside interactive
--    dispatch. Two consequences: errors must be `pcall`ed and reported
--    here (an uncaught raise inside `pmacs.async` goes to *errors*, not
--    the status line), and `pmacs.window.*` calls made after the await
--    act for the *ambient* active frontend, since interactive origin
--    does not survive the tick boundary.

-- Emacs 28's dired-kill-when-opening-new-dired-buffer, as a setting
-- rather than a hardcoded policy: buffer-per-directory accumulates
-- buffers when walking a deep tree, and Emacs users differ on whether
-- that is a feature.
pmacs.config.define {
  name = "dired.kill-when-opening",
  description = "Kill the dired buffer being left when descending or ascending.",
  type = "boolean",
  default = false,
  mutability = "live",
}

-- ---------------------------------------------------------------------------
-- Layout
-- ---------------------------------------------------------------------------
--
-- The mark column is column 0 (Q#DR4), so every other column sits two
-- bytes right of the M8.2 fixture's offsets. Stage 1 always renders it
-- blank: filling it in is Stage 2's job, but reserving it now means
-- Stage 2 does not have to move every column, and Stage 3's
-- column-classifying intercept can be written against constants that
-- did not shift under it. Offsets are computed from the widths for the
-- same reason --- the fixture hardcoded `NAME_START = 39` and paid for
-- it in every wdired test.

local MARK_BYTES = 2
local KIND_BYTES = 1
local PERMS_BYTES = 9
local SIZE_BYTES = 10
local MTIME_BYTES = 16

local MARK_START = 0
local KIND_START = MARK_START + MARK_BYTES                          -- 2
local PERMS_START = KIND_START + KIND_BYTES                         -- 3
local PERMS_END = PERMS_START + PERMS_BYTES                         -- 12 (exclusive)
local SIZE_START = PERMS_END + 1                                    -- 13
local MTIME_START = SIZE_START + SIZE_BYTES + 1                     -- 24
local NAME_START = MTIME_START + MTIME_BYTES + 1                    -- 41

local BLANK_MARK = string.rep(" ", MARK_BYTES)

local SORT_MODES = { "name", "mtime", "size" }

-- ---------------------------------------------------------------------------
-- Per-buffer state
-- ---------------------------------------------------------------------------
--
-- handles: array of { buf, path, entries, errors, sort_mode, prev }.
--
-- Keyed by linear scan over `BufferIdLua.__eq` rather than by table
-- key: two BufferIdLua values for the same buffer are distinct
-- userdata, so a `handles[buf]` lookup would miss. The scan is over a
-- handful of dired buffers. Dead buffers are compacted out first, so a
-- command in a removed dired buffer sees "not in dired" rather than
-- operating on dead state (the M8.2 fixture's `find_handle` lesson).

local handles = {}

local function live_handles()
  local live = {}
  for _, h in ipairs(handles) do
    local ok, valid = pcall(h.buf.is_valid, h.buf)
    if ok and valid then live[#live + 1] = h end
  end
  handles = live
  return live
end

local function handle_for_buffer(buf)
  if buf == nil then return nil end
  for _, h in ipairs(live_handles()) do
    if h.buf == buf then return h end
  end
  return nil
end

local function handle_for_path(path)
  for _, h in ipairs(live_handles()) do
    if h.path == path then return h end
  end
  return nil
end

local function active_handle()
  return handle_for_buffer(pmacs.window.buffer())
end

-- ---------------------------------------------------------------------------
-- Paths and names
-- ---------------------------------------------------------------------------

local canonicalize = pmacs.path.canonicalize

local function join_path(dir, name)
  if dir:sub(-1) == "/" then return dir .. name end
  return dir .. "/" .. name
end

-- Parent of a canonical directory, through the same normalizer: `..`
-- against the root folds away, so `/` is its own parent and no separate
-- root special case can drift out of agreement with the canonical form.
local function parent_path(path)
  return canonicalize(join_path(path, ".."))
end

local function basename(path)
  return path:match("([^/]+)/*$")
end

local function dirname(path)
  local dir = path:match("^(.*)/[^/]*$")
  if dir == nil then return nil end
  if dir == "" then return "/" end
  return dir
end

local function buffer_name(path)
  return "*dired:" .. path .. "*"
end

local function buffer_named(name)
  for _, id in ipairs(pmacs.buffer.list()) do
    local ok, described = pcall(pmacs.describe.buffer, id)
    if ok and described and described.name == name then return id end
  end
  return nil
end

-- The directory a prompt or a jump should start from: the active
-- buffer's own directory, else the process cwd (which the normalizer
-- yields for a bare "." because it absolutizes against it).
local function current_directory()
  local buf = pmacs.window.buffer()
  if buf ~= nil then
    local ok, path = pcall(function() return buf:path() end)
    if ok and path then
      local dir = dirname(path)
      if dir then return canonicalize(dir) end
    end
    local h = handle_for_buffer(buf)
    if h then return h.path end
  end
  return canonicalize(".")
end

-- ---------------------------------------------------------------------------
-- Failure reporting
-- ---------------------------------------------------------------------------

-- `Handle:await()` raises structured tables (R45), so `tostring` on a
-- failure yields "table: 0x...". Every user-visible dired failure goes
-- through here.
local function failure_message(err)
  if type(err) == "table" then
    return tostring(err.message or err.tag or "error")
  end
  return tostring(err)
end

local function report(where, err)
  pmacs.editor.set_status(where .. ": " .. failure_message(err))
end

-- ---------------------------------------------------------------------------
-- Rendering
-- ---------------------------------------------------------------------------

-- `rwxr-xr-x`, without the leading kind char (rendered separately so a
-- symlink shows `l` and a directory `d`). Arithmetic rather than bit
-- ops: this file has to run on LuaJIT (5.1) as well as Lua 5.4.
--
-- The nine basic bits only: setuid / setgid / sticky are deliberately
-- not surfaced as Emacs's `s` / `t`, matching the M8.3 fixture's
-- `parse_perm_string`, which edits exactly these nine. Rendering a bit
-- Stage 3 could not accept back would be worse than omitting it.
local function fmt_perms(mode)
  local function tri(bits)
    local r = (bits >= 4) and "r" or "-"
    local w = ((bits % 4) >= 2) and "w" or "-"
    local x = ((bits % 2) >= 1) and "x" or "-"
    return r .. w .. x
  end
  return tri(math.floor(mode / 64) % 8)
      .. tri(math.floor(mode / 8) % 8)
      .. tri(mode % 8)
end

local function kind_char(kind)
  if kind == "dir" then return "d"
  elseif kind == "symlink" then return "l"
  elseif kind == "file" then return "-"
  else return "?"  -- device, fifo, socket
  end
end

-- Exact bytes while they fit the column; a magnitude past that.
--
-- `%10d` holds ten digits, so a file of 10 GB or more (VM images, core
-- dumps --- ordinary things) widens the field and shifts mtime and name
-- right on that line alone. That is only cosmetic today, but
-- `_layout.NAME_START` is exported as a contract and Stage 3's
-- column-classifying intercept is planned against these constants, so a
-- line that violates them now is a Stage 3 trap. Same discipline as
-- `fmt_mtime`: the width is the invariant, and precision yields to it.
--
-- This is NOT the deferred human-readable size column (§13): the exact
-- byte count is still what a listing shows, right up to the point where
-- it cannot be shown at all.
local SIZE_UNITS = { "K", "M", "G", "T", "P", "E" }

local function fmt_size(n)
  local exact = string.format("%" .. SIZE_BYTES .. "d", n)
  if #exact <= SIZE_BYTES then return exact end
  local value, unit = n, SIZE_UNITS[#SIZE_UNITS]
  for _, suffix in ipairs(SIZE_UNITS) do
    value = value / 1024
    unit = suffix
    if value < 1024 then break end
  end
  local scaled = string.format("%.1f%s", value, unit)
  if #scaled > SIZE_BYTES then scaled = scaled:sub(1, SIZE_BYTES) end
  return string.rep(" ", SIZE_BYTES - #scaled) .. scaled
end

local function fmt_mtime(secs)
  -- Explicit format string, so the width is fixed and the result does
  -- not move with LC_TIME. A pre-epoch mtime is legal and `os.date`'s
  -- behavior on a negative time is platform-dependent, so a
  -- non-conforming result degrades to a fixed-width placeholder rather
  -- than shifting every column right of it.
  local ok, formatted = pcall(os.date, "%Y-%m-%d %H:%M", secs)
  if ok and type(formatted) == "string" and #formatted == MTIME_BYTES then
    return formatted
  end
  return string.rep("?", MTIME_BYTES)
end

-- POSIX permits any byte but `/` and NUL in a filename, including `\n`.
-- Rendering one verbatim would break the one-line-per-entry invariant
-- that cursor-line -> entry resolution rests on (and that Stage 3's
-- intercept will rest on harder), so control bytes are escaped. The
-- backslash goes first, which is what makes the encoding invertible ---
-- Stage 3 needs the exact inverse so a no-op commit cannot fire a
-- spurious rename. Carried over from the M8.2 fixture as decided
-- design, not re-litigated.
local function escape_displayable(s)
  if s == nil then return "" end
  s = s:gsub("\\", "\\\\")
  s = s:gsub("\n", "\\n")
  s = s:gsub("\r", "\\r")
  s = s:gsub("\t", "\\t")
  -- NUL is deliberately absent from the class: the kernel forbids it in
  -- a filename, so the fixture's `%z` (removed from Lua 5.2's pattern
  -- syntax) was covering a case that cannot occur.
  s = s:gsub("[\1-\8\11\12\14-\31]", function(ch)
    return string.format("\\x%02X", string.byte(ch))
  end)
  return s
end

local function render_entry(entry)
  local target = ""
  if entry.symlink_target then
    target = " -> " .. escape_displayable(entry.symlink_target)
  elseif entry.kind == "symlink" then
    -- A tolerant listing keeps a symlink whose target could not be
    -- represented (non-UTF-8) or read; say so rather than rendering a
    -- bare `l` line that looks like a complete entry.
    target = " -> ?"
  end
  return string.format(
    "%s%s%s %s %s %s%s",
    BLANK_MARK, kind_char(entry.kind), fmt_perms(entry.mode),
    fmt_size(entry.size), fmt_mtime(entry.mtime),
    escape_displayable(entry.name), target)
end

-- Header (line 0) + one line per entry + the unreadable-count footer.
-- The footer exists because a tolerant listing that silently dropped
-- entries is worse than one that failed: the user has to know the view
-- is incomplete (and Stage 3's wdired refuses to open on one).
local function render_text(handle)
  local lines = { handle.path .. ":" }
  for _, entry in ipairs(handle.entries) do
    lines[#lines + 1] = render_entry(entry)
  end
  local unreadable = #handle.errors
  if unreadable > 0 then
    lines[#lines + 1] = string.format("%d entries unreadable", unreadable)
  end
  return table.concat(lines, "\n")
end

-- Dired's own writes are the only ones that reach the buffer: the
-- read-only intercept rejects everything else, and this bypasses it.
local function paint(handle)
  local text = render_text(handle)
  handle.buf:replace(0, handle.buf:len(), text, { bypass_intercept = true })
end

-- ---------------------------------------------------------------------------
-- Cursor
-- ---------------------------------------------------------------------------
--
-- Entry i renders on line i (line 0 is the header), so the entry under
-- the cursor is `entries[cursor_line()]`.

local function entry_at_cursor(handle)
  local line = pmacs.editor.cursor_line()
  if line < 1 then return nil end
  return handle.entries[line], line
end

local function index_of_name(handle, name)
  if name == nil then return nil end
  for i, entry in ipairs(handle.entries) do
    if entry.name == name then return i end
  end
  return nil
end

-- Re-seat by BASENAME (Q#DR9), falling back to the nearest surviving
-- line. Every repaint is wholesale, so without this a revert, a sort,
-- or any Stage 2 operation would drop the cursor to the header.
--
-- `move_to_line` is AMBIENT --- it moves the active window's cursor, not
-- `handle.buf`'s --- so every caller that can run after an `:await()`
-- has to check that dired is still the active buffer first. Painting is
-- safe either way (it names the buffer); seating is not. Callers that
-- activate the buffer themselves (an open, which displays first) are
-- unconditionally in the right place.
local function seat_cursor(handle, name, fallback_line)
  local count = #handle.entries
  if count == 0 then
    pmacs.editor.move_to_line(0)
    return
  end
  local target = index_of_name(handle, name)
  if target == nil then
    target = math.max(1, math.min(fallback_line or 1, count))
  end
  pmacs.editor.move_to_line(target)
end

-- ---------------------------------------------------------------------------
-- Sorting
-- ---------------------------------------------------------------------------

local function sort_entries(entries, mode)
  if mode == "name" then
    table.sort(entries, function(a, b) return a.name < b.name end)
  elseif mode == "mtime" then
    -- Newest first, name as a stable tiebreak so a directory of
    -- same-second files renders deterministically.
    table.sort(entries, function(a, b)
      if a.mtime ~= b.mtime then return a.mtime > b.mtime end
      return a.name < b.name
    end)
  elseif mode == "size" then
    table.sort(entries, function(a, b)
      if a.size ~= b.size then return a.size > b.size end
      return a.name < b.name
    end)
  else
    error("dired: unknown sort mode: " .. tostring(mode))
  end
end

local function next_sort_mode(mode)
  for i, candidate in ipairs(SORT_MODES) do
    if candidate == mode then
      return SORT_MODES[(i % #SORT_MODES) + 1]
    end
  end
  return SORT_MODES[1]
end

-- ---------------------------------------------------------------------------
-- Reading
-- ---------------------------------------------------------------------------

-- Read and sort one directory without touching editor state, so a
-- failure happens before any side effect is committed (acceptance 15).
-- Must run inside `pmacs.async`.
--
-- Always tolerant (Q#DR6): a plain refresh of a busy directory must not
-- fail because one child was unlinked between `readdir` and `lstat`.
-- Parent-level failures and non-UTF-8 *names* still raise.
local function read_listing(path, sort_mode)
  local listing = pmacs.fs.read_dir(path, { tolerant = true }):await()
  local entries = listing.entries
  sort_entries(entries, sort_mode)
  return entries, listing.errors
end

-- ---------------------------------------------------------------------------
-- Buffer ownership
-- ---------------------------------------------------------------------------

-- How far the `<2>`, `<3>`, ... disambiguation walks before giving up.
local NAME_VARIANT_LIMIT = 99

-- `pmacs.buffer.create` takes any caller-chosen name, so a foreign
-- buffer may already be called `*dired:/tmp*`. Painting into it through
-- `bypass_intercept` would clobber a user's data, so found-by-name is
-- NOT adoption: ownership means "this buffer is in dired's own handle
-- table" (F7).
--
-- That is deliberately narrower than the framing's "in the handle table
-- OR major_mode == dired": a foreign buffer that also carries the mode
-- is precisely the case the check exists to refuse, and a builtin's
-- handle table cannot be lost the way a reloadable package's can.
local function claim_handle(path)
  local existing = handle_for_path(path)
  if existing then return existing end

  local name = buffer_name(path)
  if buffer_named(name) then
    local unique = nil
    for i = 2, NAME_VARIANT_LIMIT do
      local candidate = string.format("%s<%d>", name, i)
      if buffer_named(candidate) == nil then
        unique = candidate
        break
      end
    end
    if unique == nil then
      error(string.format("dired: %s is taken and no free variant remains", name))
    end
    name = unique
  end

  local buf = pmacs.buffer.create(name)
  -- Read-only by the listview idiom (Q#DR3): every non-bypass edit is
  -- rejected, and the intercept lives as long as the buffer.
  pmacs.buffer.add_intercept(buf, function()
    error(name .. " is read-only")
  end)
  -- Q#DR3/Q#P6: while this buffer is active a semantic frontend must
  -- round-trip keys, or optimistic apply would swallow the single-key
  -- bindings (`g` would insert a `g` into a CRDT mirror instead of
  -- reverting) and bypass the intercept entirely.
  pmacs.buffer.set_round_trip_input(buf, true)
  -- Q#DR8: the mode is what carries the keymap, and dired is #129's
  -- first consumer of mode-scoped keys outside language detection.
  pmacs.buffer.set_major_mode(buf, "dired")

  local handle = {
    buf = buf,
    path = path,
    entries = {},
    errors = {},
    sort_mode = SORT_MODES[1],
    prev = nil,
  }
  handles[#handles + 1] = handle
  return handle
end

-- ---------------------------------------------------------------------------
-- Display
-- ---------------------------------------------------------------------------

local function drop_handle(handle)
  for i, candidate in ipairs(handles) do
    if candidate == handle then
      table.remove(handles, i)
      return
    end
  end
end

-- Kill the dired buffer being left, when the user asked for it.
-- Deliberately after the new buffer is displayed: `pmacs.buffer.kill`
-- redirects windows showing the doomed buffer, and doing that first
-- would fight the display we are about to perform.
local function kill_departed(departed, arriving)
  if departed == nil or departed == arriving then return end
  if not pmacs.config.get("dired.kill-when-opening") then return end
  local ok, err = pcall(pmacs.buffer.kill, departed.buf)
  if ok then
    drop_handle(departed)
  else
    -- A buffer that could not be killed keeps its handle: dropping it
    -- would leave a live dired buffer no command recognizes.
    report("dired", err)
  end
end

-- Where a dired buffer goes.
--
-- A fresh `dired` takes the standard adopter opt (Q#BP11b): omitted or
-- "current" is the raw switch every other adopter defaults to in
-- Stages 1-2, "panel" is the bottom side window.
--
-- Navigation (`departed ~= nil`) instead reuses the window dired
-- already occupies, which is the opposite routing from a file visit and
-- deliberately so (Q#DR10): the next directory is the same kind of
-- thing as the current one and belongs in the same slot, while a file
-- is not a dired buffer and belongs in the document area.
local function display(handle, opts, departed)
  local side = nil
  if departed ~= nil then
    -- Dired's own window, not the request's: walking a tree in a side
    -- window keeps the side window.
    local params = pmacs.window.params()
    side = params and params.side
  elseif opts and opts.display == "panel" then
    side = "bottom"
  end
  if side ~= nil then
    -- A side slot DEDICATED to another buffer refuses the replacement
    -- and this falls back to the document window (Q#BP3 2.iii). That is
    -- both the substrate's documented policy and Emacs's, so dired does
    -- not try to unpin the user's panel.
    pmacs.window.display(handle.buf, { side = side, select = true })
  else
    pmacs.window.switch_buffer(handle.buf)
  end
end

-- ---------------------------------------------------------------------------
-- Public: open a directory
-- ---------------------------------------------------------------------------

pmacs.dired = pmacs.dired or {}

local OPEN_OPTS = { display = true, select_name = true }

-- Open `path`'s dired buffer, replacing `departed` (a handle) in the
-- window it occupies when this is a navigation rather than a fresh
-- open. Awaits, so it must run inside `pmacs.async`; raises on a read
-- failure, having changed nothing. Returns the buffer.
local function open_directory(path, opts, departed)
  if type(path) ~= "string" then
    error("pmacs.dired.open: path must be a string, got " .. type(path))
  end
  opts = opts or {}
  -- Validated up front, before the read and before any buffer exists,
  -- so a bad opt leaves nothing to roll back (the
  -- `parse_adopter_placement` discipline).
  for key in pairs(opts) do
    if not OPEN_OPTS[key] then
      error(string.format("pmacs.dired.open: unknown opts key %q", tostring(key)))
    end
  end
  local wanted = opts.display
  if wanted ~= nil and wanted ~= "current" and wanted ~= "panel" then
    error(string.format('pmacs.dired.open: unknown display %q (expected "current" or "panel")',
      tostring(wanted)))
  end
  local canonical = canonicalize(path)

  -- Read first: a failure must leave no buffer, no window change, and
  -- no handle behind.
  local sort_mode = (handle_for_path(canonical) or {}).sort_mode or SORT_MODES[1]
  local entries, errors = read_listing(canonical, sort_mode)

  local handle = claim_handle(canonical)
  handle.entries = entries
  handle.errors = errors
  handle.sort_mode = sort_mode

  -- `q` returns to the buffer you came from, never to another dired
  -- buffer (which would trap `q` walking back down the tree); on a
  -- descent the arriving buffer inherits the departing one's origin.
  if departed ~= nil then
    handle.prev = departed.prev
  else
    local active = pmacs.window.buffer()
    if active ~= nil and handle_for_buffer(active) == nil then
      handle.prev = active
    end
  end

  paint(handle)
  display(handle, opts, departed)
  -- Seating happens after the display: `switch_buffer` zeroes the
  -- window cursor, so an earlier seat would be discarded.
  seat_cursor(handle, opts.select_name, 1)
  kill_departed(departed, handle)
  return handle.buf
end

function pmacs.dired.open(path, opts)
  return open_directory(path, opts, nil)
end

-- Every interactive entry point funnels through here: spawn the
-- coroutine the await needs, and turn a failure into a status message
-- rather than an uncaught raise inside `pmacs.async` (which would land
-- in *errors* and leave the user with a silent no-op).
local function open_async(path, opts, departed, where)
  pmacs.async(function()
    local ok, err = pcall(open_directory, path, opts, departed)
    if not ok then report(where or "dired", err) end
  end)
end

-- ---------------------------------------------------------------------------
-- Commands
-- ---------------------------------------------------------------------------

pmacs.command.define {
  name = "dired",
  description = "Open a directory listing (dired).",
  fn = function()
    local root = current_directory()
    -- No completion source, deliberately. `source = "files"` would make
    -- RET-on-empty open whatever sorts first (the minibuffer selects
    -- candidate 0 whenever the list is non-empty, and a selected
    -- candidate shadows typed text --- S0-1/S0-4), and RET-on-the-
    -- default-directory is exactly the gesture `C-x d` exists for. The
    -- field is prefilled instead, which is Emacs's own shape here.
    pmacs.minibuffer.read {
      prompt = "Dired: ",
      initial = root,
      history = "dired",
      on_accept = function(value)
        if value == nil or value == "" then return end
        open_async(value, nil, nil, "dired")
      end,
    }
  end,
}

pmacs.command.define {
  name = "dired-jump",
  description = "Open dired on the current file's directory, cursor on that file.",
  fn = function()
    local buf = pmacs.window.buffer()
    local path = nil
    if buf ~= nil then
      local ok, value = pcall(function() return buf:path() end)
      if ok then path = value end
    end
    if path == nil then
      pmacs.editor.set_status("dired-jump: this buffer has no file")
      return
    end
    local dir = dirname(path)
    if dir == nil then
      pmacs.editor.set_status("dired-jump: cannot find the directory of " .. path)
      return
    end
    open_async(dir, { select_name = basename(path) }, nil, "dired-jump")
  end,
}

pmacs.command.define {
  name = "dired.visit",
  description = "Visit the entry under the cursor (descend a directory, open a file).",
  fn = function()
    local handle = active_handle()
    if handle == nil then return end
    local entry = entry_at_cursor(handle)
    -- The header and the unreadable-count footer are not entries.
    if entry == nil then return end
    local target = join_path(handle.path, entry.name)
    if entry.kind == "dir" then
      open_async(target, nil, handle, "dired")
      return
    end
    if entry.kind == "symlink" then
      -- `read_dir` and `stat` are both lstat-based, so nothing in the
      -- entry says whether the link points at a directory --- the only
      -- way to find out is to try to list it. A symlinked directory is
      -- an ordinary thing to walk into, so try the descent and fall back
      -- to a file visit.
      --
      -- `open_directory` is the try: it reads before touching any editor
      -- state and raises having changed nothing (acceptance 15), so its
      -- failure IS the "not a directory" answer. An explicit probe
      -- followed by the real open would list the whole directory TWICE
      -- --- opendir plus one lstat per child, each time.
      pmacs.async(function()
        local descended = pcall(open_directory, target, nil, handle)
        if descended then return end
        local visited, err = pcall(pmacs.window.display_file, target, { select = true })
        if not visited then report("dired", err) end
      end)
      return
    end
    -- Q#DR10: `display_file`, never `find_or_open`, which switches the
    -- active window in both branches before firing hooks --- in a
    -- panel-displayed dired that would replace the panel with the
    -- visited file, i.e. the panel swallows itself.
    local ok, err = pcall(pmacs.window.display_file, target, { select = true })
    if not ok then report("dired", err) end
  end,
}

pmacs.command.define {
  name = "dired.parent",
  description = "Open the parent directory.",
  fn = function()
    local handle = active_handle()
    if handle == nil then return end
    local parent = parent_path(handle.path)
    if parent == handle.path then
      pmacs.editor.set_status("dired: already at the filesystem root")
      return
    end
    -- Seat on the directory we came from, the way Emacs's `^` does.
    open_async(parent, { select_name = basename(handle.path) }, handle, "dired")
  end,
}

pmacs.command.define {
  name = "dired.revert",
  description = "Re-read the directory, keeping the cursor on its entry.",
  fn = function()
    local handle = active_handle()
    if handle == nil then return end
    local entry, line = entry_at_cursor(handle)
    local name = entry and entry.name
    pmacs.async(function()
      local ok, entries, errors = pcall(read_listing, handle.path, handle.sort_mode)
      if not ok then
        -- On failure `entries` carries the raised value, not a listing.
        report("dired", entries)
        return
      end
      if not handle.buf:is_valid() then return end
      handle.entries = entries
      handle.errors = errors
      paint(handle)
      -- The re-read settles a tick or more later, and the user may have
      -- left (a buffer switch, or `q`) in the meantime. The paint names
      -- its buffer and is safe; seating is ambient, so a stale seat here
      -- would move an unrelated buffer's cursor to a line index that
      -- only means something in this listing.
      if pmacs.window.buffer() == handle.buf then
        seat_cursor(handle, name, line)
      end
    end)
  end,
}

pmacs.command.define {
  name = "dired.sort-cycle",
  description = "Cycle the sort mode: name -> mtime -> size.",
  fn = function()
    local handle = active_handle()
    if handle == nil then return end
    local entry, line = entry_at_cursor(handle)
    local name = entry and entry.name
    -- A pure reorder of the entries already in hand: sort is a display
    -- decision, not a reason to re-read the directory.
    handle.sort_mode = next_sort_mode(handle.sort_mode)
    sort_entries(handle.entries, handle.sort_mode)
    paint(handle)
    seat_cursor(handle, name, line)
    pmacs.editor.set_status("dired: sorted by " .. handle.sort_mode)
  end,
}

pmacs.command.define {
  name = "dired.quit",
  description = "Leave dired, restoring the previous buffer.",
  fn = function()
    local handle = active_handle()
    if handle == nil then return end
    -- Q#BP11b, matching `listview.quit`: `q` keeps its name and its
    -- user-visible behavior, delegating to `window.quit` only when
    -- dired really is in a side window.
    local params = pmacs.window.params()
    if params and params.side and params.quit_action then
      pmacs.window.quit()
      return
    end
    local target = handle.prev
    if not (target and target:is_valid()) then
      target = buffer_named("*scratch*") or pmacs.buffer.create("*scratch*")
    end
    pmacs.window.switch_buffer(target)
  end,
}

-- ---------------------------------------------------------------------------
-- Keys
-- ---------------------------------------------------------------------------

-- Global: both sequences are unbound repo-wide, and both are the Emacs
-- defaults.
pmacs.keymap.bind { scope = "global", sequence = "C-x d", command = "dired" }
pmacs.keymap.bind { scope = "global", sequence = "C-x C-j", command = "dired-jump" }

-- In-buffer keys are MODE-scoped (Q#DR8), bound once here rather than
-- per buffer: a second dired buffer needs no `keymap.bind` of its own,
-- and Stage 3's wdired swap changes the whole keymap with the mode
-- instead of unbinding key by key.
local function bind(sequence, command)
  pmacs.keymap.bind { scope = "mode", mode = "dired", sequence = sequence, command = command }
end

bind("RET", "dired.visit")
bind("f", "dired.visit")
bind("^", "dired.parent")
bind("n", "cursor.down")
bind("<down>", "cursor.down")
bind("p", "cursor.up")
bind("<up>", "cursor.up")
bind("g", "dired.revert")
bind("q", "dired.quit")
bind("s", "dired.sort-cycle")

-- ---------------------------------------------------------------------------
-- Test seam
-- ---------------------------------------------------------------------------
--
-- The layout constants, so acceptance can assert column positions
-- without hardcoding the numbers this file computes.
pmacs.dired._layout = {
  MARK_START = MARK_START,
  KIND_START = KIND_START,
  PERMS_START = PERMS_START,
  PERMS_END = PERMS_END,
  SIZE_START = SIZE_START,
  MTIME_START = MTIME_START,
  NAME_START = NAME_START,
}
