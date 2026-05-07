-- pmacs-dired/init.lua --- Directory view package (T M8.2).
--
-- The dired-class entry of M8's three universality-proof packages.
-- Validates the buffer-as-projection-of-external-state shape: each
-- line of the buffer corresponds to one filesystem entry, and edits
-- to lines (M8.3, wdired) become file operations.
--
-- This v0.1 covers the read-only directory view. Wdired's
-- editable-line layer lands in T M8.3.
--
-- Public surface:
--
--   local dired = require("pmacs-dired")
--   pmacs.async(function()
--     dired.open("/home/user")
--   end)
--
--   -- inside a dired buffer:
--   --   RET     -> open subdirectory under cursor
--   --   <BS>    -> navigate to parent directory
--   --   M-x pmacs-dired.sort-name   sort by filename
--   --   M-x pmacs-dired.sort-mtime  sort by modification time (newest first)
--   --   M-x pmacs-dired.sort-size   sort by size (largest first)
--
-- Reload safety: this package is reload-safe under
-- `pmacs.packages.reload("pmacs-dired")`. The on-unload hook drops
-- per-buffer state and unregisters every command this package
-- defined, so re-running the chunk after reload doesn't hit
-- DuplicateName.

local M = {}

-- ---------------------------------------------------------------------------
-- Per-buffer state
-- ---------------------------------------------------------------------------
--
-- Each open dired buffer owns a handle: { buf, path, sort_mode, entries }.
-- We key handles by linear scan via `BufferIdLua.__eq` rather than
-- by buffer name (the name encodes the path, which navigation
-- mutates) or by raw id (BufferIdLua intentionally hides its inner
-- value via R22). The scan is over a small list --- typically 1-3
-- dired buffers per session --- so the cost is negligible.
--
-- `pmacs.buffer.remove(id)` can leave a window temporarily pointing
-- at a stale BufferId. `pmacs.window.buffer()` still returns that id,
-- and BufferId equality is raw-id equality, so a removed dired buffer
-- would otherwise still match its old handle. find_handle compacts
-- the table to live buffers before comparing; stale dired commands
-- then see "not in dired" instead of operating on dead buffer state.

local handles = {}

local function find_handle(buf)
  local live = {}
  local found = nil
  for _, h in ipairs(handles) do
    local ok, valid = pcall(h.buf.is_valid, h.buf)
    if ok and valid then
      live[#live + 1] = h
      if h.buf == buf then
        found = h
      end
    end
  end
  handles = live
  return found
end

local function active_handle()
  return find_handle(pmacs.window.buffer())
end

-- ---------------------------------------------------------------------------
-- Rendering
-- ---------------------------------------------------------------------------

-- Format mode bits as `rwxr-xr-x` (9 chars, no leading kind char).
-- The kind char is rendered separately in render_entry so symlinks
-- show as `l` and dirs as `d` per dired convention.
--
-- Uses arithmetic rather than `&` / `>>` so the package compiles
-- under both LuaJIT (Lua 5.1, no integer bit ops) and Lua 5.4.
-- Mode is in the range 0..0o7777 (< 4096); `math.floor` + `%`
-- gets us a portable octal-digit extractor.
local function fmt_perms(mode)
  local function tri(bits)
    -- bits is 0..7. High bit = 4, middle = 2, low = 1.
    local r = (bits >= 4) and "r" or "-"
    local w = ((bits % 4) >= 2) and "w" or "-"
    local x = ((bits % 2) >= 1) and "x" or "-"
    return r .. w .. x
  end
  local owner = math.floor(mode / 64) % 8
  local group = math.floor(mode / 8) % 8
  local other = mode % 8
  return tri(owner) .. tri(group) .. tri(other)
end

local function kind_char(kind)
  if kind == "dir" then return "d"
  elseif kind == "symlink" then return "l"
  elseif kind == "file" then return "-"
  else return "?"  -- device, fifo, socket, etc.
  end
end

local function fmt_size(n)
  -- Right-aligned to 10 columns. dired-class doesn't bother with
  -- human-readable units in v0.1; the package layer can format
  -- differently if a user surfaces the need.
  return string.format("%10d", n)
end

local function fmt_mtime(secs)
  -- ISO-8601-ish, minute precision. Stable across locales because
  -- os.date with an explicit format string ignores LC_TIME.
  return os.date("%Y-%m-%d %H:%M", secs)
end

-- Escape control characters that would otherwise break the
-- one-line-per-entry invariant or make the listing hard to inspect,
-- and the literal backslash that would otherwise alias with our
-- escape sequences. Unix filenames can contain `\n` and `\r`
-- (POSIX permits any byte except `/` and NUL); rendering them
-- verbatim produces a multi-line buffer entry, which would break
-- cursor-line -> entry resolution and the wdired layer's line-edit
-- detection.
--
-- The wdired layer (T M8.3) parses rendered entry text to detect
-- user edits, so the escape must be unambiguous --- a filename
-- containing the literal two characters `\` + `n` must not look
-- the same as one containing an actual newline. We therefore
-- escape `\` itself first (so a literal `\n` renders as `\\n`,
-- distinct from an actual newline's `\n`).
--
-- Other C0 controls render as `\xNN`; printable bytes, DEL, and
-- UTF-8 multibyte sequences pass through. The encoding mirrors
-- `ls --quoting=c`'s handling for the subset of characters dired
-- needs.
local function escape_displayable(s)
  if s == nil then return "" end
  -- Order matters: backslash must be escaped first so subsequent
  -- substitutions only insert the literal-backslash escapes once.
  s = s:gsub("\\", "\\\\")
  s = s:gsub("\n", "\\n")
  s = s:gsub("\r", "\\r")
  s = s:gsub("\t", "\\t")
  s = s:gsub("[%z\1-\8\11\12\14-\31]", function(ch)
    return string.format("\\x%02X", string.byte(ch))
  end)
  return s
end

-- Inverse of escape_displayable: turn a rendered (escaped) basename
-- back into the bytes the kernel sees. Returns (real, nil) on
-- success or (nil, error) on a malformed escape. Recognized
-- sequences mirror escape_displayable exactly --- `\\`, `\n`, `\r`,
-- `\t`, and `\xNN` (case-insensitive hex). Any other backslash
-- escape is rejected: the renderer never produces it, so seeing
-- one means the user typed something ambiguous, and we'd rather
-- fail at commit than silently rename to whatever bytes our
-- best-effort decoder produced.
--
-- Why this exists at the M8.3 layer (not in the renderer): commit
-- compares the user's edited line against the snapshot's *real*
-- name. Without an inverse, a file whose real name is "weird\nname"
-- renders as `weird\nname` and a no-op commit would parse the
-- rendered text as a NEW name `weird\nname` (literal backslash + n)
-- and fire a spurious rename. unescape_displayable fixes that.
local function unescape_displayable(s)
  local out = {}
  local i = 1
  local n = #s
  while i <= n do
    local b = s:byte(i)
    if b ~= 92 then  -- not '\\'
      out[#out + 1] = string.char(b)
      i = i + 1
    else
      if i == n then
        return nil, "trailing backslash with no escape character"
      end
      local nxt = s:sub(i + 1, i + 1)
      if nxt == "\\" then
        out[#out + 1] = "\\"; i = i + 2
      elseif nxt == "n" then
        out[#out + 1] = "\n"; i = i + 2
      elseif nxt == "r" then
        out[#out + 1] = "\r"; i = i + 2
      elseif nxt == "t" then
        out[#out + 1] = "\t"; i = i + 2
      elseif nxt == "x" then
        if i + 3 > n then
          return nil, ("incomplete \\xNN escape at byte " .. i)
        end
        local hex = s:sub(i + 2, i + 3)
        if not hex:match("^%x%x$") then
          return nil, ("invalid \\xNN escape '\\x" .. hex ..
                       "' at byte " .. i)
        end
        out[#out + 1] = string.char(tonumber(hex, 16))
        i = i + 4
      else
        return nil, ("unknown escape '\\" .. nxt ..
                     "' at byte " .. i ..
                     " (only \\\\, \\n, \\r, \\t, \\xNN are valid)")
      end
    end
  end
  return table.concat(out), nil
end

-- Render one entry as a single line. Symlinks append ` -> target`
-- so the user can see what each link points at; the suffix is
-- purely informational. v0.1 wdired does not support editing the
-- target (commit rejects any change to the suffix); a future
-- milestone could add a target-edit path with a separate
-- `symlink` reconfiguration syscall.
local function render_entry(e)
  local kch = kind_char(e.kind)
  local perms = fmt_perms(e.mode)
  local name = escape_displayable(e.name)
  local target = ""
  if e.symlink_target then
    target = " -> " .. escape_displayable(e.symlink_target)
  end
  return string.format(
    "%s%s %s %s %s%s",
    kch, perms, fmt_size(e.size), fmt_mtime(e.mtime),
    name, target
  )
end

-- ---------------------------------------------------------------------------
-- Sort modes
-- ---------------------------------------------------------------------------

local SORT_MODES = { "name", "mtime", "size" }

local function sort_entries(entries, mode)
  if mode == "name" then
    table.sort(entries, function(a, b) return a.name < b.name end)
  elseif mode == "mtime" then
    -- Newest first; stable tiebreak by name so renders are
    -- deterministic when two entries share an mtime (common on
    -- fresh extracts).
    table.sort(entries, function(a, b)
      if a.mtime ~= b.mtime then return a.mtime > b.mtime end
      return a.name < b.name
    end)
  elseif mode == "size" then
    -- Largest first; stable tiebreak by name.
    table.sort(entries, function(a, b)
      if a.size ~= b.size then return a.size > b.size end
      return a.name < b.name
    end)
  else
    error("pmacs-dired: unknown sort mode: " .. tostring(mode))
  end
end

-- ---------------------------------------------------------------------------
-- Buffer rendering
-- ---------------------------------------------------------------------------

-- Build the full buffer text: header line (current path + ":")
-- followed by one entry per line. Returns the rendered string.
local function render_text(handle)
  local lines = { handle.path .. ":" }
  for _, e in ipairs(handle.entries) do
    lines[#lines + 1] = render_entry(e)
  end
  return table.concat(lines, "\n")
end

-- Replace the buffer's current contents with the rendered text.
-- Used by every operation that changes the displayed listing
-- (initial open, navigate, sort). The `painting` flag is set so a
-- wdired intercept (M8.3) can let the package's own writes pass
-- through without re-validating each as a user edit.
--
-- The full buffer-touching operation runs inside a pcall'd closure
-- so the flag is guaranteed to clear before any error propagates.
-- This includes `handle.buf:len()` --- the closure form is
-- specifically necessary because Lua evaluates pcall's argument
-- list eagerly: `pcall(fn, a, b, buf:len())` evaluates `buf:len()`
-- *before* entering the protected scope, so a `:len()` failure
-- (e.g., on a buffer that was removed via pmacs.buffer.remove
-- between paint() entries) would skip past the flag-clear and
-- leave the wdired intercept in passthrough mode for subsequent
-- user edits --- silently disabling validation. M8.4 audit
-- finding 2 (same shape M6.9 finding 7 called out for the REPL
-- package's `_self_write` flag).
local function paint(handle)
  local text = render_text(handle)
  handle.painting = true
  local ok, err = pcall(function()
    handle.buf:replace(0, handle.buf:len(), text)
  end)
  handle.painting = false
  if not ok then error(err) end
end

-- ---------------------------------------------------------------------------
-- Reading the directory
-- ---------------------------------------------------------------------------

-- Read and sort a directory without mutating editor state. Both
-- initial open and later navigation use this so read failures happen
-- before any side effects are committed.
local function read_entries(path, sort_mode)
  local entries = pmacs.fs.read_dir(path):await()
  sort_entries(entries, sort_mode)
  return entries
end

-- Navigate `handle` to `target_path`: read the directory first,
-- then atomically commit `path`, `entries`, and the buffer paint.
-- If `read_dir(target_path):await()` fails, the handle and buffer
-- remain pointing at the previous location --- which is the
-- correct dired-class behavior for a navigation that couldn't
-- complete. (The earlier shape mutated `handle.path` first and
-- left the buffer showing the old listing under a header for the
-- new path on read failure, breaking the line-per-entry contract
-- and confusing a subsequent sort.)
--
-- Errors propagate from :await() as the standard
-- `{ tag = "failed", message = ... }` structured value, which the
-- caller can pcall around. Must run inside `pmacs.async(...)`.
local function navigate_to(handle, target_path)
  local entries = read_entries(target_path, handle.sort_mode)
  handle.path = target_path
  handle.entries = entries
  paint(handle)
end

-- Re-read the current path (no navigation). Same atomic guarantee.
local function refresh(handle)
  navigate_to(handle, handle.path)
end

-- ---------------------------------------------------------------------------
-- Public: open a directory
-- ---------------------------------------------------------------------------

-- Open `path` in a fresh dired buffer. Must be called from inside
-- a `pmacs.async(...)` body because read_dir is async. Returns
-- the handle (mostly for testing; user code rarely needs it).
--
-- The directory is read before any editor-visible side effects. If
-- read_dir fails, no empty dired buffer is created and no stale
-- handle is registered.
--
-- `path` must be absolute. parent_path() and the open-line
-- navigation both assume absolute paths; accepting relative paths
-- would silently produce wrong parents (e.g. parent of `.` -> `/`).
-- The caller can resolve via `pmacs.fs.realpath` (when that
-- ships) or just pass an absolute string.
function M.open(path)
  if type(path) ~= "string" then
    error("pmacs-dired.open: path must be a string, got " .. type(path))
  end
  if path:sub(1, 1) ~= "/" then
    error("pmacs-dired.open: path must be absolute (start with '/'); got: " .. path)
  end

  local sort_mode = "name"
  local entries = read_entries(path, sort_mode)

  local buf = pmacs.buffer.create("*dired:" .. path .. "*")
  local handle = {
    buf = buf,
    path = path,
    sort_mode = sort_mode,
    entries = entries,
  }
  handles[#handles + 1] = handle

  paint(handle)
  pmacs.window.switch_buffer(buf)
  pmacs.keymap.bind {
    scope = "buffer", buffer = buf, sequence = "RET",
    command = "pmacs-dired.open-line",
  }
  pmacs.keymap.bind {
    scope = "buffer", buffer = buf, sequence = "Backspace",
    command = "pmacs-dired.parent",
  }

  return handle
end

-- ---------------------------------------------------------------------------
-- Path manipulation
-- ---------------------------------------------------------------------------

-- Compute the parent of an absolute path. `/foo/bar` -> `/foo`,
-- `/foo` -> `/`, `/` -> `/` (root is its own parent --- the user
-- can't navigate further up).
local function parent_path(path)
  if path == "/" then return "/" end
  -- Strip trailing slash if any (defensive: path could've been
  -- canonicalised by the user differently).
  local p = path:gsub("/$", "")
  -- Greedy `/[^/]*` match strips the last segment.
  local stripped = p:gsub("/[^/]*$", "")
  if stripped == "" then return "/" end
  return stripped
end

-- Join `dir` and `name` into an absolute path. `/foo` + `bar`
-- -> `/foo/bar`; `/` + `bar` -> `/bar`.
local function join_path(dir, name)
  if dir:sub(-1) == "/" then return dir .. name end
  return dir .. "/" .. name
end

-- ---------------------------------------------------------------------------
-- Cursor -> entry resolution
-- ---------------------------------------------------------------------------

-- Map the active window's cursor line to the entry on that line.
-- Line 0 is the header; lines >= 1 are entries[1..]. Returns the
-- entry table or nil if the cursor is on the header (or out of
-- range).
local function entry_at_cursor(handle)
  local line = pmacs.editor.cursor_line()
  if line < 1 then return nil end
  return handle.entries[line]
end

-- ---------------------------------------------------------------------------
-- Wdired layer (T M8.3): editable buffer maps to rename / chmod
-- ---------------------------------------------------------------------------
--
-- A dired buffer has two modes:
--
--   * read-only (default): no intercept; navigation commands work;
--     the buffer is not user-writable in the normal sense (paint()
--     overwrites it on every refresh, so any user edit would be
--     clobbered).
--
--   * wdired-edit (this section): an intercept_edit chain entry is
--     attached to the buffer that constrains user edits to two
--     editable column ranges per entry line --- the perms column
--     (bytes 1..10 of the line) and the name column (bytes 39..end-
--     of-line). Every other position is read-only and rejected at
--     intercept time. Edits within the perms region must come from
--     the rwx alphabet, also rejected at intercept time.
--
-- Render layout per entry line, byte by byte:
--
--   col 0              kind char      ('d', 'l', '-', '?')   -- ro
--   col 1..9 (incl)    perms (9 ch)   ("rwxr-xr-x")           -- editable
--   col 10             space          (" ")                   -- ro
--   col 11..20 (incl)  size %10d      ("       1234")         -- ro
--   col 21             space                                   -- ro
--   col 22..37 (incl)  mtime          ("2026-05-05 12:34")    -- ro
--   col 38             space                                   -- ro
--   col 39..eol        name           ("foo.txt")             -- editable
--
-- (Symlinks have ` -> target` appended in the name column. The
-- editable region runs to end-of-line for files and directories;
-- on a symlink line, only the basename portion is meaningfully
-- editable --- the trailing ` -> target` suffix and the perms
-- column are both gated by commit-time / intercept-time checks
-- because chmod follows symlinks and target reconfiguration is
-- out of scope for v0.1.)
--
-- Commit translates buffer state → filesystem ops:
--   - name changed  → pmacs.fs.rename(old_path, new_path)
--   - perms changed → pmacs.fs.chmod(path, new_mode)
--
-- Renames are routed through unique temp names within the same
-- directory (two-phase) so swaps and chains commit safely without
-- per-pair ordering analysis; `rename(2)` would otherwise replace
-- existing targets atomically.
--
-- External changes between edit-toggle and commit are detected by
-- re-reading the directory and comparing every snapshot field
-- (count, names, mode, kind, size, mtime seconds+nsec, symlink
-- target) against the current state. Any divergence aborts the
-- commit with a message pointing at `pmacs-dired.wdired-abandon`,
-- which discards
-- pending edits and refreshes the listing from disk.

-- Layout constants ---------------------------------------------------------

local KIND_BYTES = 1
local PERMS_BYTES = 9
local SIZE_BYTES = 10
local MTIME_BYTES = 16
-- Computed offsets within an entry line (start positions, inclusive).
local PERMS_START = KIND_BYTES                                     -- 1
local PERMS_END = PERMS_START + PERMS_BYTES                        -- 10 (exclusive)
local NAME_START = PERMS_END + 1 + SIZE_BYTES + 1 + MTIME_BYTES + 1 -- 39

-- Permission-character validation ------------------------------------------

local function is_perm_byte(b)
  return b == 45  -- '-'
      or b == 114 -- 'r'
      or b == 119 -- 'w'
      or b == 120 -- 'x'
end

local function bytes_are_all_perm_chars(s)
  for i = 1, #s do
    if not is_perm_byte(s:byte(i)) then return false end
  end
  return true
end

-- Parse a 9-char permission string into a numeric mode. Returns the
-- mode integer (0..0o777) on success, or nil + reason on failure.
-- The check is *positional*: byte 0 must be 'r' or '-', byte 1 must
-- be 'w' or '-', byte 2 must be 'x' or '-', repeating per triple.
-- Setuid / setgid / sticky bits are not surfaced --- v0.1 wdired
-- only edits the basic 9 mode bits.
local function parse_perm_string(s)
  if #s ~= 9 then
    return nil, ("perms column must be exactly 9 chars; got " .. #s)
  end
  local mode = 0
  local triples = { { 64, 32, 16 }, { 8, 4, 2 }, { 4, 2, 1 } }
  -- triples[i] = bit values for triple i; we'll compute owner/group/other separately
  -- Owner (bytes 0..2) -> 0o400, 0o200, 0o100
  -- Group (bytes 3..5) -> 0o040, 0o020, 0o010
  -- Other (bytes 6..8) -> 0o004, 0o002, 0o001
  local bit_values = { 256, 128, 64, 32, 16, 8, 4, 2, 1 }
  local expected = { "r", "w", "x", "r", "w", "x", "r", "w", "x" }
  for i = 1, 9 do
    local ch = s:sub(i, i)
    if ch == expected[i] then
      mode = mode + bit_values[i]
    elseif ch ~= "-" then
      return nil, ("invalid perms char '" .. ch .. "' at position " ..
                   tostring(i) .. " (expected '" .. expected[i] .. "' or '-')")
    end
  end
  return mode
end

-- Buffer line resolution ---------------------------------------------------

-- Find the entry index for an absolute byte position. Returns
-- (idx, line_start_pos, line_end_pos_exclusive) or nil if pos is in
-- the header or past the last entry. line_end_pos_exclusive points
-- to the trailing '\n' (or buf:len() for the last entry).
local function entry_at_byte(handle, pos)
  local edit = handle.edit
  if not edit then return nil end
  local marks = edit.line_start_marks
  local n = #marks
  if n == 0 then return nil end
  -- Linear scan: typical wdired use is small (dozens of entries).
  -- For a 10K-entry dired the user typically wouldn't go into
  -- wdired-edit; if they do, scan cost is still negligible compared
  -- to the user's typing speed.
  local last_idx = nil
  local last_start = nil
  for i = 1, n do
    local s = marks[i]:get()
    if pos < s then break end
    last_idx, last_start = i, s
  end
  if not last_idx then return nil end
  local line_end
  if last_idx < n then
    line_end = marks[last_idx + 1]:get() - 1  -- the '\n' position
  else
    line_end = handle.buf:len()
  end
  return last_idx, last_start, line_end
end

-- Classify a writable region by absolute byte range. `start_pos` is
-- inclusive; `end_pos` is exclusive (the half-open convention used
-- by EditOp). Returns "perms" or "name" if the range fits entirely
-- in one writable column of an entry line; returns nil if the range
-- crosses lines, lands in a read-only column, or would change the
-- perms-column width.
--
-- Perms is fixed-width (9 chars). Inserts and deletes inside it
-- shift adjacent columns and would force every consumer (the
-- intercept body, the commit parser, the line-reader) to handle
-- variable-width perms. We forbid that at the edge: a zero-length
-- range (insert) NEVER classifies as "perms" --- inserts are valid
-- only inside the variable-width name column. Deletes and replaces
-- can land in perms; the caller (the intercept body) enforces
-- length preservation for those.
local function classify_writable_range(handle, start_pos, end_pos)
  local zero_len = (start_pos == end_pos)
  local idx, line_start, line_end = entry_at_byte(handle, start_pos)
  if not idx then return nil end
  -- The end of the range must be in the same line.
  if start_pos > line_end or end_pos > line_end then
    return nil
  end
  local rel_start = start_pos - line_start
  local rel_end = end_pos - line_start
  -- name region: [NAME_START, eol). Inserts and deletes are
  -- length-changing here (variable-width column); that's fine.
  if rel_start >= NAME_START then
    return "name"
  end
  -- perms region: [PERMS_START, PERMS_END). Inserts disallowed
  -- (they'd extend perms past 9 chars). Deletes / replaces must
  -- stay strictly inside the column.
  if not zero_len and rel_start >= PERMS_START and rel_end <= PERMS_END then
    return "perms"
  end
  return nil
end

-- The intercept body itself --------------------------------------------------

-- Reject any perms-column edit that lands on a symlink line. The
-- core fs surface documents that chmod follows symlinks (per
-- `chmod(2)`); on most filesystems a symlink reports 0o777
-- regardless of mode bits applied to it, and a chmod through the
-- link would silently mutate the *target* file's mode. The user's
-- displayed perms come from lstat (the link's metadata), so they
-- can't tell which file they're about to modify. Rather than
-- offering a knob with surprising semantics, v0.1 wdired makes
-- the perms column on a symlink line read-only at intercept time.
-- A future milestone could surface lchmod (where supported) or
-- offer an explicit "edit target perms" command.
local function reject_if_symlink_perms(handle, range_start, op_label)
  local idx = entry_at_byte(handle, range_start)
  if idx and handle.edit.snapshot[idx].kind == "symlink" then
    error("pmacs-dired wdired: " .. op_label .. " on a symlink line's " ..
          "perms column is not supported (chmod follows symlinks; " ..
          "editing the displayed link perms would silently mutate the " ..
          "target file's mode while leaving the link's lstat perms " ..
          "unchanged --- v0.1 wdired only edits names on symlink lines)")
  end
end

local function make_intercept_body(handle)
  return function(op)
    -- Pass through the package's own paint operations.
    if handle.painting then return nil end
    -- If the buffer is mid-edit teardown (commit or abandon flipped
    -- handle.edit to nil but the intercept is still attached for a
    -- frame), fail closed: reject the edit. The caller can retry
    -- once the intercept is detached.
    if not handle.edit then
      error("pmacs-dired: edit teardown in progress; retry after commit/abandon completes")
    end
    local kind = op.kind
    if kind == "insert" then
      local pos = op.pos
      local bytes = op.bytes or ""
      -- Reject any insert containing a newline --- it would split a
      -- single entry line into two physical lines, breaking the
      -- one-line-per-entry contract.
      if bytes:find("\n", 1, true) then
        error("pmacs-dired wdired: newline insertions are not allowed " ..
              "(would split entry line " .. tostring(pos) .. ")")
      end
      local region = classify_writable_range(handle, pos, pos)
      if region == nil then
        error("pmacs-dired wdired: insert at byte " .. tostring(pos) ..
              " is in a read-only column (perms is fixed-width 9 chars " ..
              "and accepts only same-length replaces; only the name " ..
              "column accepts inserts)")
      end
      -- region is "name"; classify_writable_range never returns
      -- "perms" for a zero-length range.
    elseif kind == "delete" then
      local s, e = op.start, op["end"]
      local region = classify_writable_range(handle, s, e)
      if region == nil then
        error("pmacs-dired wdired: deletion of bytes [" .. tostring(s) ..
              ", " .. tostring(e) .. ") crosses or enters a read-only column " ..
              "(only the perms and name columns are editable)")
      end
      if region == "perms" then
        error("pmacs-dired wdired: deletes inside the perms column are " ..
              "not allowed (perms is a fixed-width 9-char field; use a " ..
              "same-length replace to change permission bits)")
      end
    elseif kind == "replace" then
      local s, e = op.start, op["end"]
      local bytes = op.bytes or ""
      if bytes:find("\n", 1, true) then
        error("pmacs-dired wdired: replacement bytes contain a newline " ..
              "(would split entry line)")
      end
      local region = classify_writable_range(handle, s, e)
      if region == nil then
        error("pmacs-dired wdired: replacement of bytes [" .. tostring(s) ..
              ", " .. tostring(e) .. ") crosses or enters a read-only column")
      end
      if region == "perms" then
        reject_if_symlink_perms(handle, s, "replace")
        local range_len = e - s
        if #bytes ~= range_len then
          error("pmacs-dired wdired: perms column edits must preserve the " ..
                "9-char width; got replacement of " .. tostring(range_len) ..
                " bytes with " .. tostring(#bytes) .. " bytes")
        end
        if not bytes_are_all_perm_chars(bytes) then
          error("pmacs-dired wdired: perms column accepts only 'r', 'w', " ..
                "'x', and '-'; got " .. string.format("%q", bytes))
        end
      end
    end
    return nil  -- pass-through for accepted edits
  end
end

-- Edit-mode entry / exit ----------------------------------------------------

-- Snapshot the entries and place a mark at each entry line's start.
-- `painting` must be false when this runs (we don't want our own
-- paint to be re-validated by the intercept we're about to attach).
local function setup_edit_state(handle)
  local edit = {
    snapshot = {},
    line_start_marks = {},
    intercept = nil,
  }
  -- Header is "<path>:" + "\n". The first entry's line starts at
  -- byte (#header_text + 1) where header_text = handle.path .. ":".
  local pos = #handle.path + 1 + 1
  for i, e in ipairs(handle.entries) do
    -- Defensive deep copy of the fields we rely on at commit. The
    -- snapshot is what we diff edits against; it must not shift
    -- under us if anything else mutates handle.entries[i].
    edit.snapshot[i] = {
      name = e.name,
      mode = e.mode,
      kind = e.kind,
      size = e.size,
      mtime = e.mtime,
      mtime_nsec = e.mtime_nsec or 0,
      symlink_target = e.symlink_target,
    }
    edit.line_start_marks[i] = pmacs.buffer.mark_create(
      handle.buf, pos, { gravity = "right" }
    )
    pos = pos + #render_entry(e) + 1  -- +1 for the trailing \n
  end
  return edit
end

local function teardown_edit_state(handle)
  local edit = handle.edit
  if not edit then return end
  if edit.intercept then
    pmacs.buffer.remove_intercept(edit.intercept)
    edit.intercept = nil
  end
  for _, mark in ipairs(edit.line_start_marks) do
    mark:remove()
  end
  handle.edit = nil
end

-- Read the current rendered text of entry line i (1-indexed).
-- Returns the full line text (without the trailing newline).
local function read_line_text(handle, idx)
  local edit = handle.edit
  local marks = edit.line_start_marks
  local s = marks[idx]:get()
  local e
  if idx < #marks then
    e = marks[idx + 1]:get() - 1  -- before the '\n'
  else
    e = handle.buf:len()
  end
  return handle.buf:slice(s, e)
end

-- Parse one rendered line into its (perms_str, name) pair, OR
-- return nil + reason if the line is malformed for commit purposes.
-- We expect: kind(1) + perms(9) + " " + size(10) + " " + mtime(16)
-- + " " + name(rest). Spaces at the fixed offsets must still be
-- spaces (the intercept enforces that the user can't have edited
-- them, but we re-verify here as a defense-in-depth check before
-- producing rename ops).
local function parse_committed_line(line)
  if #line < NAME_START then
    return nil, "line too short for an entry (header was edited?)"
  end
  if line:byte(PERMS_END + 1) ~= 32  -- byte 10 must be ' '
     or line:byte(PERMS_END + 1 + SIZE_BYTES + 1) ~= 32  -- byte 21
     or line:byte(NAME_START) ~= 32 then  -- byte 38
    return nil, "fixed-width separators between columns were modified"
  end
  local perms_str = line:sub(PERMS_START + 1, PERMS_END)  -- 1-indexed slice
  local name = line:sub(NAME_START + 1)  -- everything from byte 39 onward
  return perms_str, name
end

-- Decode a rendered (escaped) name from the buffer back into the
-- real basename bytes. Returns (real_basename, nil) on success or
-- (nil, error) on a malformed line.
--
-- For a symlink, the rendered line is "<basename> -> <target>". We
-- enforce that the user didn't change <target>: v0.1 supports
-- name renames and chmods, not symlink target reconfiguration.
-- (If the user wants to repoint a symlink, they must remove + re-
-- create it; the M8.3 spec doesn't promise a target-edit path.)
-- Fail loudly here rather than silently drop the user's target
-- edit, which is what the previous implementation did.
--
-- We strip the *expected* trailing suffix (computed from the
-- snapshot's target) rather than splitting at the first ` -> `:
-- a basename containing the literal ` -> ` (which is a perfectly
-- legal Unix filename) renders as `a -> b -> target`, and the
-- naive first-arrow split would mistake `b -> target` for a
-- target edit. Suffix-stripping makes the round-trip work
-- regardless of internal arrows.
--
-- For non-symlinks the rendered name is just the escaped basename;
-- we run it through unescape_displayable so that real \n / \t /
-- \xNN bytes round-trip without producing spurious renames.
local function decode_committed_name(rendered_name, snap)
  local basename = rendered_name
  if snap.kind == "symlink" then
    local expected_suffix = " -> " ..
                            escape_displayable(snap.symlink_target or "")
    local sl = #expected_suffix
    if #rendered_name < sl or rendered_name:sub(-sl) ~= expected_suffix then
      return nil, ("symlink target edits are not supported in v0.1 " ..
                   "(expected the line to end with '" .. expected_suffix ..
                   "'); restore the original target text and retry, or " ..
                   "remove + re-create the symlink outside dired")
    end
    basename = rendered_name:sub(1, #rendered_name - sl)
  end
  local real_basename, uerr = unescape_displayable(basename)
  if not real_basename then
    return nil, uerr
  end
  return real_basename, nil
end

-- Detect external changes since edit-toggle. Re-reads the
-- directory and compares each entry against the snapshot field-by-
-- field: count, names, mode, kind, size, mtime seconds+nsec,
-- symlink target.
-- Returns nil on no change or a string error on first divergence.
--
-- Why all fields, not just names: the previous implementation
  -- compared only count + name set, so an external chmod, an
  -- external truncate, a same-size rewrite, or a kind switch (file
  -- replaced by symlink with the same name) would slip through and
  -- the commit would happily overwrite or misrepresent the
  -- post-external-change state.
local function detect_external_changes(handle)
  local current
  local ok, err = pcall(function()
    current = pmacs.fs.read_dir(handle.path):await()
  end)
  if not ok then
    return "could not re-read '" .. handle.path .. "' to verify: " .. tostring(err)
  end
  local snapshot = handle.edit.snapshot
  if #current ~= #snapshot then
    return ("entry count changed from " .. #snapshot .. " to " .. #current ..
            " (external add/remove since edit started)")
  end
  -- Build name -> snapshot-entry map for O(1) lookup.
  local snap_by_name = {}
  for _, e in ipairs(snapshot) do snap_by_name[e.name] = e end
  for _, c in ipairs(current) do
    local s = snap_by_name[c.name]
    if not s then
      return ("external rename or replacement detected: '" .. c.name ..
              "' was not present at edit-start")
    end
    if c.mode ~= s.mode then
      return ("external mode change on '" .. c.name ..
              "': was 0o" .. string.format("%o", s.mode) ..
              ", now 0o" .. string.format("%o", c.mode))
    end
    if c.kind ~= s.kind then
      return ("external kind change on '" .. c.name ..
              "': was '" .. tostring(s.kind) ..
              "', now '" .. tostring(c.kind) .. "'")
    end
    if c.size ~= s.size then
      return ("external size change on '" .. c.name ..
              "': was " .. tostring(s.size) ..
              ", now " .. tostring(c.size))
    end
    if c.mtime ~= s.mtime or (c.mtime_nsec or 0) ~= (s.mtime_nsec or 0) then
      return ("external mtime change on '" .. c.name ..
              "' (file was rewritten or touched)")
    end
    if (c.symlink_target or "") ~= (s.symlink_target or "") then
      return ("external symlink target change on '" .. c.name ..
              "': was '" .. tostring(s.symlink_target) ..
              "', now '" .. tostring(c.symlink_target) .. "'")
    end
  end
  return nil
end

-- ---------------------------------------------------------------------------
-- Commands
-- ---------------------------------------------------------------------------
--
-- We track every name we register so the on_unload hook can hand
-- the slots back to the registry. Without this, a second chunk run
-- (install_local replacement, reload) hits DuplicateName.

local OWNED_COMMANDS = {}

local function define_owned(spec)
  pmacs.command.define(spec)
  OWNED_COMMANDS[#OWNED_COMMANDS + 1] = spec.name
end

-- Reject navigation while a wdired session is open. The user has
-- pending edits that would be silently discarded by a refresh.
local function ensure_not_editing(h, op_name)
  if h and h.edit then
    error("pmacs-dired: " .. op_name .. " is not available while wdired-edit " ..
          "is active; commit (M-x pmacs-dired.wdired-commit) or abandon " ..
          "(M-x pmacs-dired.wdired-abandon) first.")
  end
end

define_owned {
  name = "pmacs-dired.open-line",
  description = "Open the directory under the cursor in the active dired buffer.",
  fn = function()
    local h = active_handle()
    if not h then return end
    ensure_not_editing(h, "pmacs-dired.open-line")
    local entry = entry_at_cursor(h)
    if not entry then return end

    if entry.kind == "dir" then
      -- Navigate: hand the target to navigate_to, which reads
      -- before committing path/entries/paint. Failure here leaves
      -- the buffer showing the current listing.
      local target = join_path(h.path, entry.name)
      pmacs.async(function()
        navigate_to(h, target)
      end)
    else
      -- Files / symlinks: opening these requires a buffer-from-file
      -- primitive that pmacs hasn't exposed yet. We could synthesize
      -- one by reading the contents into a fresh buffer, but the
      -- editor's file-load path also does encoding detection, mode
      -- selection, and dirty-tracking --- reproducing that surface
      -- inside this package would duplicate logic destined for the
      -- core. v0.1 errors with a clear message; the wdired layer
      -- (T M8.3) revisits this.
      error(
        "pmacs-dired.open-line: opening files from dired requires the " ..
        "buffer-from-file API (not yet exposed). Use the editor's " ..
        "file-open command (typically C-x C-f) to open '" ..
        join_path(h.path, entry.name) .. "' for now."
      )
    end
  end,
}

define_owned {
  name = "pmacs-dired.parent",
  description = "Navigate the active dired buffer to the parent directory.",
  fn = function()
    local h = active_handle()
    if not h then return end
    ensure_not_editing(h, "pmacs-dired.parent")
    local target = parent_path(h.path)
    pmacs.async(function()
      navigate_to(h, target)
    end)
  end,
}

-- Sort commands. Each sets the handle's mode and re-paints from
-- the cached entry list (no fresh read_dir --- sort is a pure
-- in-memory reorder). The async wrapper is for ergonomic
-- consistency with the other dired commands; sort itself doesn't
-- yield.

local function make_sort_command(mode)
  return function()
    local h = active_handle()
    if not h then return end
    ensure_not_editing(h, "pmacs-dired.sort-" .. mode)
    h.sort_mode = mode
    sort_entries(h.entries, mode)
    paint(h)
  end
end

define_owned {
  name = "pmacs-dired.sort-name",
  description = "Sort the active dired buffer alphabetically by filename.",
  fn = make_sort_command("name"),
}

define_owned {
  name = "pmacs-dired.sort-mtime",
  description = "Sort the active dired buffer by modification time, newest first.",
  fn = make_sort_command("mtime"),
}

define_owned {
  name = "pmacs-dired.sort-size",
  description = "Sort the active dired buffer by size, largest first.",
  fn = make_sort_command("size"),
}

define_owned {
  name = "pmacs-dired.wdired-edit",
  description = "Toggle the active dired buffer into editable wdired mode.",
  fn = function()
    local h = active_handle()
    if not h then
      error("pmacs-dired.wdired-edit: not in a dired buffer")
    end
    if h.edit then
      error("pmacs-dired.wdired-edit: this buffer is already in wdired-edit mode")
    end
    h.edit = setup_edit_state(h)
    h.edit.intercept = pmacs.buffer.add_intercept(
      h.buf, make_intercept_body(h)
    )
  end,
}

define_owned {
  name = "pmacs-dired.wdired-abandon",
  description = "Discard wdired edits and refresh the dired listing from disk.",
  fn = function()
    local h = active_handle()
    if not h or not h.edit then return end
    -- Detach intercept first so paint() isn't blocked by it. The
    -- immediate paint() repaints from the cached snapshot so the
    -- user instantly sees a non-edit view; the async navigate_to
    -- below then re-reads the directory so any external changes
    -- (which is precisely the case wdired-commit's error
    -- guidance points users here for) are reflected. If the
    -- async refresh fails, the cached paint stays visible and
    -- the failure surfaces via the modeline.
    teardown_edit_state(h)
    paint(h)
    pmacs.async(function()
      local ok, err = pcall(navigate_to, h, h.path)
      if not ok and pmacs.editor and pmacs.editor.set_status then
        pmacs.editor.set_status(
          "pmacs-dired.wdired-abandon: refresh failed: " .. tostring(err)
        )
      end
    end)
  end,
}

-- The synchronous (inside-async) commit body. Raises on any error;
-- the wdired-commit command wraps this in pcall so the outcome can
-- be observed via `handle.last_commit_outcome` even when the work
-- was scheduled by a fire-and-forget pmacs.async.
local function do_wdired_commit(h, progress)
  progress = progress or { disk_touched = false }
  -- Bullet 4: detect external changes before producing ops. Any
  -- mismatch (count, names, mode, kind, size, mtime, symlink
  -- target) aborts the commit before we touch disk.
  local changed = detect_external_changes(h)
  if changed then
    error(changed ..
          "; run pmacs-dired.wdired-abandon to discard your edits " ..
          "and refresh the listing from disk")
  end

  -- Phase 1: read every line, parse, decode, validate, and build
  -- per-entry chmod / rename plans. We collect all ops first and
  -- only execute after every line has parsed cleanly --- a
  -- malformed line means the commit aborts with no syscalls.
  local snapshot = h.edit.snapshot
  local final_names = {}  -- index-aligned with snapshot
  local chmods = {}        -- list of { from_basename, mode }
  local renames = {}       -- list of { from_basename, to_basename }
  for i, snap in ipairs(snapshot) do
    local line = read_line_text(h, i)
    local perms_str, rendered_name = parse_committed_line(line)
    if not perms_str then
      error("line " .. tostring(i) ..
            " (entry '" .. snap.name .. "') is malformed: " ..
            tostring(rendered_name))
    end
    local mode, perr = parse_perm_string(perms_str)
    if not mode then
      error("line " .. tostring(i) ..
            " (entry '" .. snap.name .. "') has invalid perms: " .. perr)
    end
    local real_basename, derr = decode_committed_name(rendered_name, snap)
    if not real_basename then
      error("line " .. tostring(i) ..
            " (entry '" .. snap.name .. "'): " .. derr)
    end
    if real_basename == "" then
      error("line " .. tostring(i) ..
            " (entry '" .. snap.name .. "') has an empty name")
    end
    if real_basename:find("/", 1, true) then
      error("line " .. tostring(i) ..
            " (entry '" .. snap.name .. "') name contains '/' " ..
            "(directory separator not allowed in a basename)")
    end
    -- POSIX names cannot contain NUL, but unescape_displayable
    -- happily accepts \x00 escapes. If we let a NUL through, the
    -- chmod-then-rename ordering would chmod the file, then the
    -- rename syscall would fail at the kernel boundary --- a
    -- partial commit. Reject here, before any syscall fires.
    if real_basename:find("\0", 1, true) then
      error("line " .. tostring(i) ..
            " (entry '" .. snap.name .. "') name contains a NUL byte " ..
            "(POSIX filenames cannot contain NUL)")
    end
    final_names[i] = real_basename
    -- Preserve snapshot's high bits (setuid/setgid/sticky and the
    -- filetype bits stat returned). parse_perm_string only sets
    -- the low 9 bits; OR them in.
    local snap_high_bits = math.floor(snap.mode / 512) * 512
    local target_mode = snap_high_bits + mode
    if target_mode ~= snap.mode then
      chmods[#chmods + 1] = { from_basename = snap.name, mode = target_mode }
    end
    if real_basename ~= snap.name then
      renames[#renames + 1] = { from_basename = snap.name,
                                to_basename = real_basename }
    end
  end

  -- Reject duplicate final names: two entries can't end up named
  -- the same. Catches both the "rename a -> b where b is unchanged"
  -- overwrite case and the "two lines have the same edited name"
  -- typo. (The swap case --- a -> b, b -> a --- has unique final
  -- names; that's handled below by the temp-name two-phase rename.)
  do
    local seen = {}
    for i, name in ipairs(final_names) do
      local prev = seen[name]
      if prev then
        error("commit aborted: line " .. tostring(i) ..
              " (entry '" .. snapshot[i].name ..
              "') would collide with line " .. tostring(prev) ..
              " (entry '" .. snapshot[prev].name ..
              "'); both have final name '" .. name .. "'")
      end
      seen[name] = i
    end
  end

  -- Run chmods on the snapshot's old paths first. After this
  -- block, the file's mode bits are updated; the rename phase
  -- below moves the now-modified file to its final name (if it
  -- has a rename pending). The opposite order would either chmod
  -- a non-existent old path or chmod the new path before the
  -- user's rename intent has been recorded, both surprising.
  for _, c in ipairs(chmods) do
    pmacs.fs.chmod(join_path(h.path, c.from_basename), c.mode):await()
    progress.disk_touched = true
  end

  -- Two-phase rename: route every rename through a unique temp
  -- name in the same directory. This makes swaps (a -> b, b -> a)
  -- and chains (a -> b, b -> c, c -> a) safe without a per-pair
  -- analysis. POSIX rename() replaces an existing target
  -- atomically, so a direct rename(a, b) where b is in the
  -- snapshot would silently destroy b; the temp-name detour
  -- avoids that entirely. The cost is one extra rename per
  -- renamed entry, which on an in-directory rename is a constant-
  -- time path-table tweak --- negligible.
  if #renames > 0 then
    -- Build a unique prefix that doesn't collide with any
    -- snapshot or planned final name.
    local prefix = ".pmacs-wdired-tmp-" ..
                   tostring(os.time()) .. "-" ..
                   tostring(math.random(1, 1000000))
    local plen = #prefix
    local function prefix_taken()
      for _, snap in ipairs(snapshot) do
        if snap.name:sub(1, plen) == prefix then return true end
      end
      for _, name in ipairs(final_names) do
        if name:sub(1, plen) == prefix then return true end
      end
      return false
    end
    if prefix_taken() then
      error("internal: temp-name prefix '" .. prefix ..
            "' collides with an entry in '" .. h.path ..
            "'; rename the colliding entry outside dired and retry")
    end
    -- Phase 1: each renamed source -> a unique temp name.
    for i, r in ipairs(renames) do
      r.tmp_basename = prefix .. "-" .. tostring(i)
      pmacs.fs.rename(
        join_path(h.path, r.from_basename),
        join_path(h.path, r.tmp_basename)
      ):await()
      progress.disk_touched = true
    end
    -- Phase 2: each temp -> its final name. Ordering is irrelevant
    -- now: every target name is unique (collision check above) and
    -- no source overlaps any target (sources are temp names).
    for _, r in ipairs(renames) do
      pmacs.fs.rename(
        join_path(h.path, r.tmp_basename),
        join_path(h.path, r.to_basename)
      ):await()
      progress.disk_touched = true
    end
  end

  -- Tear down edit state. We *don't* refresh from disk here ---
  -- that's the wdired-commit command's job, after observing that
  -- we returned without raising. Reason: a refresh failure (e.g.,
  -- the parent directory was removed between our last syscall and
  -- the refresh read_dir) must not flip a successfully-applied
  -- commit to outcome="failed", because the disk changes are real
  -- and the user's intent was honored. The command splits those
  -- two outcomes into "ok" and "applied; refresh failed: ...".
  teardown_edit_state(h)
end

define_owned {
  name = "pmacs-dired.wdired-commit",
  description = "Apply wdired edits as filesystem rename / chmod ops and refresh.",
  fn = function()
    -- Outer-shape contract: this command can be invoked from any
    -- context (M-x dispatch, keybinding, test eval). It schedules a
    -- pmacs.async that does the real work. The outcome is reported
    -- two ways:
    --
    --   1. handle.last_commit_outcome --- nil while pending, "ok"
    --      on success, "failed: <msg>" on validation / pre-syscall
    --      failure, "partially applied: <msg>; ..." when at least
    --      one filesystem op landed before a later op failed, or
    --      "applied; refresh failed: <msg>" when disk ops succeeded
    --      but the cosmetic refresh failed. This is what automated
    --      tests poll.
    --   2. pmacs.editor.set_status (when available) --- a one-line
    --      modeline message. This is what interactive users see.
    --
    -- pmacs.async surfaces uncaught coroutine errors via
    -- pmacs.error rather than re-raising to the caller, so a bare
    -- pcall around invoke() wouldn't observe a commit failure.
    -- Wrapping the work in pcall *inside* the async body and
    -- writing the outcome to the handle is the canonical way for
    -- async ops to report results back to non-async callers.
    local h = active_handle()
    if not h or not h.edit then
      error("pmacs-dired.wdired-commit: not in wdired-edit mode")
    end
    if h.commit_pending then
      error("pmacs-dired.wdired-commit: commit already in progress")
    end
    h.last_commit_outcome = nil
    h.commit_pending = true
    pmacs.async(function()
      -- Phase A: validation + syscalls. A failure here means the
      -- commit was aborted at planning (before any syscall) or
      -- partway through the rename/chmod batch. Once any syscall has
      -- succeeded, a later failure is not a clean rejection: report
      -- it as partial application, tear down edit mode, and try to
      -- refresh so the buffer reflects disk.
      local progress = { disk_touched = false }
      local ok, err = pcall(do_wdired_commit, h, progress)
      if not ok then
        local outcome
        if progress.disk_touched then
          if h.edit then
            teardown_edit_state(h)
          end
          local ok_refresh, refresh_err = pcall(navigate_to, h, h.path)
          outcome = "partially applied: " .. tostring(err)
          if ok_refresh then
            outcome = outcome .. "; refreshed"
          else
            outcome = outcome .. "; refresh failed: " .. tostring(refresh_err)
          end
        else
          outcome = "failed: " .. tostring(err)
        end
        h.last_commit_outcome = outcome
        h.commit_pending = false
        if pmacs.editor and pmacs.editor.set_status then
          pmacs.editor.set_status("pmacs-dired.wdired-commit: " .. outcome)
        end
        return
      end
      -- Phase B: refresh the buffer from disk. The disk side has
      -- already settled; this read is purely cosmetic. Don't fold
      -- a refresh failure into "failed:" --- that would tell the
      -- user their changes were rejected when in fact they're
      -- already applied. Use a distinct outcome so callers (and
      -- tests) can tell the two apart.
      local ok2, err2 = pcall(navigate_to, h, h.path)
      if ok2 then
        h.last_commit_outcome = "ok"
        h.commit_pending = false
        if pmacs.editor and pmacs.editor.set_status then
          pmacs.editor.set_status("pmacs-dired.wdired-commit: applied")
        end
      else
        h.last_commit_outcome = "applied; refresh failed: " .. tostring(err2)
        h.commit_pending = false
        if pmacs.editor and pmacs.editor.set_status then
          pmacs.editor.set_status(
            "pmacs-dired.wdired-commit: applied (refresh failed: " ..
            tostring(err2) .. ")"
          )
        end
      end
    end)
  end,
}

-- ---------------------------------------------------------------------------
-- Cleanup on unload
-- ---------------------------------------------------------------------------
--
-- Reload-time hook: drop all handles, then unregister every command
-- this package owns. The buffers themselves are owned by the
-- editor; we don't try to destroy them. The next `M.open(...)` call
-- after reload starts with a fresh handle list, and the second
-- chunk run re-defines the commands without colliding.

pmacs.packages.on_unload(function()
  -- Tear down any active wdired sessions so their intercepts don't
  -- linger on buffers after the package is reloaded.
  for _, h in ipairs(handles) do
    if h.edit then
      teardown_edit_state(h)
    end
  end
  handles = {}
  for _, name in ipairs(OWNED_COMMANDS) do
    -- pmacs.command.unregister returns a bool, never errors on
    -- missing names. Discard the bool: a missing command means
    -- something else already cleared it, which is fine.
    pmacs.command.unregister(name)
  end
  OWNED_COMMANDS = {}
end)

-- ---------------------------------------------------------------------------
-- Test seam
-- ---------------------------------------------------------------------------
--
-- The acceptance suite needs to query a handle's state without
-- relying on cursor placement (the v0.1 buffer surface doesn't
-- expose move_to_byte yet, so tests can't reliably position the
-- cursor on a specific line). The `_test` table is convention-
-- private: not stable, not documented, and not intended for other
-- packages. v0.1 audit lint does not enforce field-level privacy on
-- an exported module table, so external authors must treat this as a
-- "do not use" test seam until a stricter lint or test-only export
-- mechanism exists.

M._test = {
  active_handle = active_handle,
  parent_path = parent_path,
  sort_modes = SORT_MODES,
  escape_displayable = escape_displayable,
  unescape_displayable = unescape_displayable,
  navigate_to = navigate_to,
  paint = paint,
  -- Wdired internals (T M8.3) that the acceptance suite exercises
  -- directly so it doesn't have to drive every code path through
  -- the command surface.
  parse_perm_string = parse_perm_string,
  parse_committed_line = parse_committed_line,
  decode_committed_name = decode_committed_name,
  is_perm_byte = is_perm_byte,
  PERMS_START = PERMS_START,
  PERMS_END = PERMS_END,
  NAME_START = NAME_START,
}

return M
