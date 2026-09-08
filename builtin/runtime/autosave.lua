-- autosave.lua --- periodic recovery copies + crash recovery (Arc 3 phase 3).
--
-- Every `interval_ms` a sweep writes a private recovery copy of each
-- modified file buffer. If pmacs dies, the next session notices the copy
-- and says so; `M-x recover-file` installs it. Emacs's `auto-save-mode`
-- + `recover-file`.
--
-- The Rust half (`pmacs.autosave._*`) does the sweep and the
-- external-change guard; this file owns the cadence, the configurable
-- interval, and the UX.
--
-- On by default. Configure from init.lua:
--     pmacs.autosave.interval_ms(60000)   -- default 30000, floor 1000
--     pmacs.autosave.enable(false)        -- turn it off entirely
--
-- Recovery files live under `$XDG_STATE_HOME/pmacs/autosave/` (0700 dir,
-- 0600 files), are deleted when the buffer is saved or killed, and
-- survive a crash or a quit with unsaved changes.
--
-- Framing: docs/archive/framings/autosave-recovery-framing.md.

pmacs.autosave = pmacs.autosave or {}

local DEFAULT_INTERVAL_MS = 30000  -- Emacs's auto-save-timeout
local MIN_INTERVAL_MS = 1000       -- each sweep fsyncs; don't storm

-- Migrated to `autosave.interval-ms` (Q#CR8's third adopter: integer,
-- validated, re-read live). The tick below re-reads the registry every
-- frame — see its comment — so storage moves there instead of a
-- module-local, but the wrapper's shape and its lenient coercion (F4)
-- stay exactly as they were.
pmacs.config.define {
  name = "autosave.interval-ms",
  description = "Milliseconds between periodic autosave sweeps.",
  type = "integer",
  default = DEFAULT_INTERVAL_MS,
  min = MIN_INTERVAL_MS,
  mutability = "live",
}

local enabled = true
local last_sweep_ms = nil
-- Report on the first tick (the startup scan), and after every load.
local needs_report = true

-- enable(on) --- turn autosave off (or back on).
function pmacs.autosave.enable(on)
  enabled = (on ~= false)
  return enabled
end

-- interval_ms([ms]) --- getter when `ms` is nil, else a validated setter.
-- Shape follows `pmacs.async_config.frame_target_ms`. The tick re-reads
-- the registry every frame, so a change -- through this wrapper OR a
-- direct `pmacs.config.set` -- takes effect immediately, no restart.
-- Legacy coercion stays lenient (F4): floor a fractional `ms` FIRST,
-- then hand the registry an already-conforming integer, since
-- `pmacs.config.set` itself demands exactness. The floor-then-min error
-- message and threshold are unchanged.
function pmacs.autosave.interval_ms(ms)
  if ms == nil then return pmacs.config.get("autosave.interval-ms") end
  if type(ms) ~= "number" or ms ~= ms or ms < MIN_INTERVAL_MS then
    error("pmacs.autosave.interval_ms: expected a number >= " .. MIN_INTERVAL_MS)
  end
  local floored = math.floor(ms)
  pmacs.config.set("autosave.interval-ms", floored)
  return floored
end

-- sweep() --- force a pass now. Returns (written, blocked, conflicted).
--   blocked    --- an unclaimed crash recovery sits at the buffer's key.
--                  Overwriting it would destroy exactly what autosave
--                  protects; recovering or discarding it resumes autosave.
--   conflicted --- another buffer already owns that file's recovery slot.
--                  Recovery files are keyed by path, so when two buffers
--                  visit one file only the first can be protected.
function pmacs.autosave.sweep()
  if not enabled then return 0, 0, 0 end
  local written, blocked, conflicted = pmacs.autosave._sweep()
  if blocked and blocked > 0 then
    pmacs.editor.set_status(
      "autosave paused for " .. blocked .. " file(s) with unclaimed recovery"
        .. " --- M-x recover-file or M-x discard-recovery")
  elseif conflicted and conflicted > 0 then
    pmacs.editor.set_status(
      "autosave paused for " .. conflicted .. " buffer(s): another buffer"
        .. " is visiting the same file")
  end
  return written, blocked, conflicted
end

local function basename(path)
  return path:match("[^/]+$") or path
end

-- The last sweep failure we reported, so a persistent fault (ENOSPC, a
-- read-only state dir) logs once but keeps warning in the status line.
local last_error = nil

-- Run a sweep, surfacing any failure. `write_private` can fail --- a full
-- disk, a permission change, a clobbered state dir --- and this is a
-- data-protection feature: silently swallowing the error would leave the
-- user believing their work is safe when nothing is being written.
local function sweep_reporting()
  local ok, err = pcall(pmacs.autosave.sweep)
  if ok then
    last_error = nil
    return
  end
  local msg = "autosave FAILED: " .. tostring(err)
  pmacs.editor.set_status(msg .. " --- your work is NOT being protected")
  -- Log once per distinct fault; the status line keeps nagging every sweep.
  if msg ~= last_error then
    last_error = msg
    if pmacs.error then pcall(pmacs.error, msg) end
  end
end

-- One aggregate message however many files are recoverable. N synchronous
-- `after-load` fires (a desktop restore) collapse into a single report.
local function report_pending()
  local fresh = pmacs.autosave._pending()
  local n = #fresh
  if n == 1 then
    pmacs.editor.set_status(basename(fresh[1]) .. " has autosave recovery --- M-x recover-file")
  elseif n > 1 then
    pmacs.editor.set_status(n .. " files have autosave recovery --- M-x recover-file")
  end
  -- Corrupt copies are counted but stay quiet: a malformed recovery file
  -- must not make startup noisy. `M-x discard-recovery` removes them.
end

-- The cadence (Q#AS2). `process.after-tick` fires every frame -- and the
-- run loops tick on a frame *timeout*, not only on input, so this keeps
-- running while the editor is idle. Costs one clock read, one registry
-- get (a borrowed/copied scalar, no Lua table built -- Q#CR15) and a
-- compare per frame, and parks no worker thread (a long `workers.sleep`
-- would hold one of only `available_parallelism - 1` pool threads).
pmacs.hook.add("process.after-tick", function()
  if needs_report then
    needs_report = false
    pcall(report_pending)
  end
  if not enabled then return end
  local now = pmacs.editor.monotonic_ms()
  if last_sweep_ms == nil then
    last_sweep_ms = now
    return
  end
  if now - last_sweep_ms >= pmacs.config.get("autosave.interval-ms") then
    last_sweep_ms = now
    sweep_reporting()
  end
end)

-- A load may reveal a recovery file; report on the next tick (never from
-- inside the hook -- a desktop restore fires this once per leaf).
pmacs.hook.add("buffer.after-load", function()
  needs_report = true
  -- A kill retires the recovery copy. There is no global kill hook, so
  -- register per buffer. `_discard_buffer` is keyed by BufferId, not by a
  -- path captured here: after a rename the buffer's recovery lives under
  -- a different key than the path it loaded with. Buffers that fire no
  -- after-load (argv `[new file]`) are covered by the sweep-time GC.
  local buf = pmacs.window.buffer()
  if not buf then return end
  pcall(pmacs.buffer.on_removed, buf, function(dead)
    pcall(pmacs.autosave._discard_buffer, dead or buf)
  end)
end)

-- A clean save retires the recovery copy. Keyed by buffer, so a renamed
-- buffer's real recovery key (written under the *old* path) is removed
-- too, not just the current path's.
pmacs.hook.add("buffer.after-save", function()
  local buf = pmacs.window.buffer()
  if buf then pcall(pmacs.autosave._discard_buffer, buf) end
end)

-- A final synchronous sweep on quit: async ticks stop after this, so a
-- quit with unsaved changes must capture them here. A failure here means
-- the quit is about to discard work that was never written anywhere, so
-- it is reported rather than swallowed. Returns nil -- before-quit is
-- short-circuit and this must never veto.
pmacs.hook.add("editor.before-quit", function()
  sweep_reporting()
end)

pmacs.command.define {
  name = "recover-file",
  description = "Replace this buffer with its autosave recovery copy.",
  fn = function()
    local path = pmacs.editor.file_path()
    if not path then
      pmacs.editor.set_status("recover-file: not visiting a file")
      return
    end
    local st = pmacs.autosave._status(path)
    if st == "none" then
      pmacs.editor.set_status("recover-file: no autosave recovery for this file")
      return
    end
    if st == "corrupt" then
      pmacs.editor.set_status("recover-file: recovery file is corrupt --- M-x discard-recovery")
      return
    end
    local warn = ""
    if st == "stale" then
      warn = " [WARNING: file changed on disk since the autosave]"
    end
    -- Pin to the exact BUFFER we started on, not merely its path: two
    -- buffers can visit the same path (`pmacs.buffer.from_file` does not
    -- dedup), so a path check alone could recover into the wrong one.
    local origin_buf = pmacs.window.buffer()
    if not origin_buf then
      pmacs.editor.set_status("recover-file: no buffer")
      return
    end
    pmacs.minibuffer.read {
      prompt = "Recover from autosave?" .. warn .. " (yes/no): ",
      source = function() return { "yes", "no" } end,
      on_accept = function(answer)
        if answer ~= "yes" then
          pmacs.editor.set_status("recover-file: cancelled")
          return
        end
        -- Focus can drift while the prompt is up, and recovering into the
        -- wrong buffer is unrecoverable.
        local buf = pmacs.window.buffer()
        if buf ~= origin_buf or pmacs.editor.file_path() ~= path then
          pmacs.editor.set_status("recover-file: buffer changed; aborted")
          return
        end
        local bytes = pmacs.autosave._recover_bytes(path)
        if not bytes then
          pmacs.editor.set_status("recover-file: recovery unreadable")
          return
        end
        buf:replace(0, buf:len(), bytes)
        -- The crash data now lives in the buffer, so the copy is no
        -- longer irreplaceable: claim it (Q#AS12). Claiming by BUFFER,
        -- right after the replace, records the recovery under this
        -- buffer's id at the revision whose contents it holds -- which
        -- un-blocks autosave for the path AND lets a later kill retire
        -- the copy (a removal callback runs after the buffer is gone,
        -- with no path left to read).
        pmacs.autosave._adopt(buf)
        -- The mutators notify windows and queue CRDT but do NOT fire
        -- `buffer.after-edit` --- that comes from dispatch_key's
        -- post-command revision check, which the minibuffer shadow
        -- returns before. Fire it so LSP didChange and the syntax
        -- reparse see the recovered contents.
        pmacs.hook.run("buffer.after-edit")
        pmacs.editor.set_status("recovered from autosave --- save to keep it")
      end,
    }
  end,
}

pmacs.command.define {
  name = "discard-recovery",
  description = "Delete this file's autosave recovery copy.",
  fn = function()
    local path = pmacs.editor.file_path()
    if not path then
      pmacs.editor.set_status("discard-recovery: not visiting a file")
      return
    end
    pmacs.autosave._discard(path)
    pmacs.editor.set_status("discarded autosave recovery")
  end,
}
