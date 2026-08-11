-- builtin/runtime/fs.lua --- T M8.1 filesystem primitives surface.
--
-- Wraps the raw `pmacs._async._dispatch_fs_*` primitives in
-- handle-returning APIs that match the rest of the worker surface
-- (pmacs.workers.sleep, pmacs.workers.compute_sum, etc.).
--
-- Public surface:
--   pmacs.fs.read_dir(path [, opts])
--     Returns a Handle. :await() yields a Lua array of entry tables:
--       { name=, kind=, size=, mtime=, mtime_nsec=, mode=, symlink_target= }
--     `kind` is one of "file" / "dir" / "symlink" / "other".
--     `symlink_target` is present only on symlink entries.
--     `opts` may contain `supersede = "<key>"` to chain into the M3
--     supersede semantics (a later read_dir under the same key
--     cancels the earlier one), and `tolerant = true` to swap the
--     all-or-nothing listing for `{ entries = ..., errors = ... }`
--     (see fs.read_dir's own comment below). Any other key is an
--     error rather than being silently ignored.
--
--   Order: entries are returned in *filesystem iteration order*,
--   which is whatever the kernel's `readdir` syscall returns. On
--   ext4 / btrfs this is hash-based, not name-sorted; on tmpfs it's
--   typically insertion order. dired-class packages (M8.2) own
--   user-facing sort modes (by name, mtime, size) per the spec ---
--   the fs primitive intentionally doesn't impose an order so the
--   package layer can pick one and not pay for sorting twice.
--
--   Path / name encoding: pmacs.fs requires UTF-8 names in v0.1. A
--   directory containing a non-UTF-8 entry produces a `failed`
--   status from :await() with a structured error naming the parent
--   and the offending raw bytes, rather than silently mangling the
--   name. Byte-preserving paths are post-v0.1 work.
--
-- Each fs primitive's worker observes its CancellationToken and
-- exits with a structured `{ tag = "cancelled" }` error from
-- :await() on cancel; callers either let it propagate (the typical
-- behavior under supersede) or pcall around it.
--
--   pmacs.fs.watch(path, callback [, opts])
--     Polling file watcher. Calls callback({ kind = "changed" |
--     "created" | "removed", path = path, recursive = bool }) when
--     the snapshot changes. opts.interval_ms defaults to 250;
--     opts.recursive defaults to false. Returns a watcher with
--     :cancel() and :is_cancelled().

local async_mod = pmacs._async
assert(async_mod, "pmacs._async must be installed before pmacs.fs loads")

-- We need the Handle class from the async builtin. The `pmacs.async`
-- table exposes it via the new_handle helper indirectly; the simplest
-- shape is to build a thin local handle wrapper that mirrors the
-- async.lua one. To avoid duplicating the whole class, we go through
-- pmacs.workers.dispatch's existing handle factory.
--
-- In practice: pmacs.workers.* and pmacs.fs.* both produce handles
-- with the same surface (await/cancel/on_complete), but they're
-- created by different Lua code paths. The async.lua module exposes
-- a `_new_handle(id)` factory we reuse here; if it isn't present
-- (very early bootstrap), we synthesize a minimal handle.

local function build_handle(id)
  if pmacs.workers and pmacs.workers._new_handle then
    return pmacs.workers._new_handle(id)
  end
  -- Fallback: replicate the minimum the async.lua file exposes.
  -- This branch fires only if pmacs.fs loads before pmacs.workers,
  -- which the editor's load order doesn't permit; the assert at the
  -- top of fs.lua guarantees pmacs._async exists, and async.lua
  -- depends on the same primitive.
  error("pmacs.fs: pmacs.workers._new_handle missing; did async.lua load before fs.lua?")
end

local fs = {}

-- Shared read-op opts parser; raises on misshapen opts.
--
-- Unknown keys are REJECTED, not ignored. The earlier version read
-- `opts.supersede` and silently dropped everything else, which means a
-- typo'd `tolerant` would degrade to the fatal contract with no signal
-- at all --- exactly the failure the tolerant opt exists to prevent
-- (dired framing §8, minor c). `allowed` is the per-op whitelist.
local function read_opts(opts, where, allowed)
  if opts == nil then return nil, false end
  if type(opts) ~= "table" then
    error(where .. ": opts must be a table or nil, got " .. type(opts))
  end
  for key in pairs(opts) do
    if not allowed[key] then
      local names = {}
      for name in pairs(allowed) do names[#names + 1] = name end
      table.sort(names)
      error(string.format("%s: unknown opts key %q (expected one of: %s)",
        where, tostring(key), table.concat(names, ", ")))
    end
  end
  local key = opts.supersede
  if key ~= nil and type(key) ~= "string" then
    error(where .. ": opts.supersede must be a string")
  end
  local tolerant = opts.tolerant
  if tolerant ~= nil and type(tolerant) ~= "boolean" then
    error(where .. ": opts.tolerant must be a boolean")
  end
  return key, tolerant == true
end

local READ_DIR_OPTS = { supersede = true, tolerant = true }
local STAT_OPTS = { supersede = true }
local WALK_TREE_OPTS = { supersede = true }

-- Two result shapes, chosen by `opts.tolerant` (dired Q#DR6):
--
--   read_dir(path)                     -> { <entry>, ... }
--   read_dir(path, { tolerant = true }) -> { entries = { <entry>, ... },
--                                            errors  = { { name = ...?,
--                                                          message = ... }, ... } }
--
-- The bare array is the M8.1 contract and stays exactly as it was, so
-- an existing consumer (the frozen M8.2 dired fixture consumes it with
-- `ipairs`) is unaffected. Under the opt, a per-entry `readdir` /
-- `lstat` / `readlink` failure and a non-UTF-8 symlink *target* become
-- `errors` rows instead of failing the whole listing; a failure on the
-- parent directory, and a non-UTF-8 entry *name*, stay fatal. An
-- `errors` row has no `name` when the entry never materialized.
function fs.read_dir(path, opts)
  if type(path) ~= "string" then
    error("pmacs.fs.read_dir: path must be a string, got " .. type(path))
  end
  local key, tolerant = read_opts(opts, "pmacs.fs.read_dir", READ_DIR_OPTS)
  local id = async_mod._dispatch_fs_read_dir(path, key, tolerant)
  return build_handle(id)
end

-- walk_tree(base [, opts]) -> handle; await -> { <entry>, ... } where
-- each entry is the read_dir shape but `name` is a BASE-RELATIVE path
-- ("sub/dir/file.txt") and the listing covers the whole tree as ONE
-- job (issue #233 D3). Symlinks are recorded, never traversed; an
-- unreadable subdirectory is skipped with its subtree; only the root
-- failing to open fails the walk. Directory entries are included
-- (kind "dir") --- consumers that only want files filter on kind.
function fs.walk_tree(base, opts)
  if type(base) ~= "string" then
    error("pmacs.fs.walk_tree: base must be a string, got " .. type(base))
  end
  local key = read_opts(opts, "pmacs.fs.walk_tree", WALK_TREE_OPTS)
  local id = async_mod._dispatch_fs_walk_tree(base, key)
  return build_handle(id)
end

function fs.stat(path, opts)
  if type(path) ~= "string" then
    error("pmacs.fs.stat: path must be a string, got " .. type(path))
  end
  local key = read_opts(opts, "pmacs.fs.stat", STAT_OPTS)
  local id = async_mod._dispatch_fs_stat(path, key)
  return build_handle(id)
end

-- Mutating fs ops (rename / chmod / remove) intentionally do NOT
-- accept opts.supersede.
--
-- pmacs.fs.chmod follows symlinks: chmodding a symlink path
-- changes the *target's* mode, per chmod(2). This is asymmetric
-- with read_dir / stat (which use lstat and report the link's
-- own mode). dired/wdired authors should be aware: a chmod issued
-- on a symlink line and then a stat refresh shows the link's
-- (unchanged) mode --- the change took effect on the target. v0.1
-- doesn't expose lchmod-style "modify the link itself" because it
-- isn't portable across Unixes; it can land later if a real
-- package needs it.
--
-- The supersede semantics on read ops cancel an in-flight predecessor
-- so only the latest result reaches Lua --- safe because no disk
-- state has changed. For mutating ops, the underlying syscall has
-- a single observable instant (it either ran and changed disk, or
-- it didn't); cancelling "before" the syscall is a race the worker
-- can't reliably win. Exposing supersede here would be misleading:
-- a "cancelled" op might still have completed.
--
-- If a package needs at-most-one-pending semantics for mutations,
-- it should serialize on the package side (await each op before
-- dispatching the next). The fs primitive can't enforce that.
--
-- **And that is a CORRECTNESS precondition, not only a cancellation
-- one (dired Stage 2a, Q#DR29).** A successful `rename` or `remove`
-- reconciles the editor's path owners in the main-thread drain — buffer
-- paths and names, the URI-keyed LSP state, the `resource.renamed` /
-- `resource.deleted` hooks. That reconciliation is deliberately
-- order-INDEPENDENT: the runtime drains the reply bus with `try_recv`
-- and establishes no execution token, so a worker can finish first and
-- be descheduled before sending, and reply order therefore does not
-- recover filesystem execution order.
--
-- Independent mutations commute, so nothing is owed for them. But
-- **mutations whose source/target paths overlap must be serialized by
-- dispatching the next only after the previous handle settles.** There
-- is no static ordering rule that would substitute: rename `dir` ->
-- `newdir` racing delete `dir/child.txt` needs delete-then-rename if
-- the delete ran first on disk and rename-then-delete if the rename
-- did, and a fixed "deletes before renames" rule gets one of the two
-- wrong — the kill misses, the rename then rebinds the buffer onto a
-- path whose file is gone, and it survives pointing at nothing.
--
-- A caller that ignores this owns the residue: a buffer left bound to a
-- stale path, or killed when it should have been rebound. Recoverable
-- and visible, not data loss — but real.

function fs.rename(from, to)
  if type(from) ~= "string" then
    error("pmacs.fs.rename: from must be a string, got " .. type(from))
  end
  if type(to) ~= "string" then
    error("pmacs.fs.rename: to must be a string, got " .. type(to))
  end
  return build_handle(async_mod._dispatch_fs_rename(from, to))
end

function fs.chmod(path, mode)
  if type(path) ~= "string" then
    error("pmacs.fs.chmod: path must be a string, got " .. type(path))
  end
  if type(mode) ~= "number" or mode < 0 or mode > 0xfff then
    error("pmacs.fs.chmod: mode must be a number in [0, 07777]")
  end
  return build_handle(async_mod._dispatch_fs_chmod(path, math.floor(mode)))
end

function fs.remove(path)
  if type(path) ~= "string" then
    error("pmacs.fs.remove: path must be a string, got " .. type(path))
  end
  return build_handle(async_mod._dispatch_fs_remove(path))
end

local Watch = {}
Watch.__index = Watch

function Watch:cancel()
  self.cancelled = true
  if self._sleep_handle then
    self._sleep_handle:cancel()
  end
end

function Watch:is_cancelled()
  return self.cancelled
end

local function join_path(base, name)
  if base:sub(-1) == "/" then return base .. name end
  return base .. "/" .. name
end

local function error_string(err)
  if type(err) == "table" then
    return tostring(err.tag or "error") .. ":" .. tostring(err.message or "")
  end
  return tostring(err)
end

local function entry_signature(path, entry)
  return table.concat({
    path,
    tostring(entry.kind),
    tostring(entry.size),
    tostring(entry.mtime),
    tostring(entry.mtime_nsec),
    tostring(entry.mode),
    tostring(entry.symlink_target or ""),
  }, "|")
end

local function append_snapshot(parts, path, recursive)
  local ok, entry = pcall(function() return fs.stat(path):await() end)
  if not ok then
    parts[#parts + 1] = path .. "|error|" .. error_string(entry)
    return false
  end
  parts[#parts + 1] = entry_signature(path, entry)
  if entry.kind ~= "dir" then
    return true
  end

  local ok_dir, entries = pcall(function() return fs.read_dir(path):await() end)
  if not ok_dir then
    parts[#parts + 1] = path .. "|read_dir_error|" .. error_string(entries)
    return true
  end

  table.sort(entries, function(a, b) return a.name < b.name end)
  for _, child in ipairs(entries) do
    local child_path = join_path(path, child.name)
    parts[#parts + 1] = entry_signature(child_path, child)
    if recursive and child.kind == "dir" then
      append_snapshot(parts, child_path, true)
    end
  end
  return true
end

local function snapshot(path, recursive)
  local parts = {}
  local exists = append_snapshot(parts, path, recursive)
  table.sort(parts)
  return table.concat(parts, "\n"), exists
end

function fs.watch(path, callback, opts)
  if type(path) ~= "string" then
    error("pmacs.fs.watch: path must be a string, got " .. type(path))
  end
  if type(callback) ~= "function" then
    error("pmacs.fs.watch: callback must be a function, got " .. type(callback))
  end
  if opts ~= nil and type(opts) ~= "table" then
    error("pmacs.fs.watch: opts must be a table or nil, got " .. type(opts))
  end
  opts = opts or {}
  local interval_ms = opts.interval_ms or 250
  if type(interval_ms) ~= "number" or interval_ms < 1 then
    error("pmacs.fs.watch: opts.interval_ms must be a positive number")
  end
  interval_ms = math.floor(interval_ms)
  local recursive = opts.recursive == true

  local watch = setmetatable({
    path = path,
    recursive = recursive,
    cancelled = false,
    _sleep_handle = nil,
  }, Watch)

  pmacs.async(function()
    local previous, previous_exists = snapshot(path, recursive)
    while not watch.cancelled do
      local sleep_handle = pmacs.workers.sleep(interval_ms)
      watch._sleep_handle = sleep_handle
      pcall(function() sleep_handle:await() end)
      watch._sleep_handle = nil
      if watch.cancelled then break end

      local current, current_exists = snapshot(path, recursive)
      if current ~= previous then
        local kind = "changed"
        if previous_exists and not current_exists then
          kind = "removed"
        elseif not previous_exists and current_exists then
          kind = "created"
        end
        previous = current
        previous_exists = current_exists
        local ok, err = pcall(callback, {
          kind = kind,
          path = path,
          recursive = recursive,
        })
        if not ok and pmacs.error then
          pmacs.error("pmacs.fs.watch callback failed: " .. tostring(err))
        elseif not ok then
          error(err)
        end
      end
    end
  end)

  return watch
end

-- pmacs.fs.canonicalize(path) -> string | nil
--
-- Arc 8 Stage 3a (framing Q#LN20). The **only synchronous** function on
-- this module, and deliberately so: its consumer is a function-valued
-- `pmacs.lsp.config[lang].root`, invoked from `ensure_server` <-
-- `attach_buffer` <- the `buffer.after-load` hook, where there is no
-- coroutine and therefore nothing to `:await()` on. Every other
-- primitive here returns a Handle; this one cannot, or it would be
-- unusable at the one call site that needs it — the same trap
-- `pmacs.fs.stat` falls into for that caller.
--
-- Resolves symlinks and `.` / `..`, returning an absolute path, or nil
-- if the path does not exist or cannot be resolved. Nil is a normal
-- answer, not an error: callers routinely ask about paths that may have
-- been deleted.
--
-- Why it exists: a configured LSP root reaches `file_uri_for` verbatim
-- and that URI is the server-affinity key (PR #161), so one project
-- opened through a symlink and through its real path would otherwise
-- spawn two servers. `pmacs.editor.file_path()` collapses `.` and `..`
-- lexically but leaves symlinks intact, so the resolver cannot get a
-- canonical path any other way.
fs.canonicalize = pmacs._fs.canonicalize

pmacs.fs = fs
