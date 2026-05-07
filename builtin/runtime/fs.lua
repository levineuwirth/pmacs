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
--     cancels the earlier one).
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

-- Shared opts.supersede extractor; raises on misshapen opts.
local function supersede_key(opts, where)
  if opts == nil then return nil end
  if type(opts) ~= "table" then
    error(where .. ": opts must be a table or nil, got " .. type(opts))
  end
  local k = opts.supersede
  if k ~= nil and type(k) ~= "string" then
    error(where .. ": opts.supersede must be a string")
  end
  return k
end

function fs.read_dir(path, opts)
  if type(path) ~= "string" then
    error("pmacs.fs.read_dir: path must be a string, got " .. type(path))
  end
  local id = async_mod._dispatch_fs_read_dir(path, supersede_key(opts, "pmacs.fs.read_dir"))
  return build_handle(id)
end

function fs.stat(path, opts)
  if type(path) ~= "string" then
    error("pmacs.fs.stat: path must be a string, got " .. type(path))
  end
  local id = async_mod._dispatch_fs_stat(path, supersede_key(opts, "pmacs.fs.stat"))
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

pmacs.fs = fs
