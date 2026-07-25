-- builtin/runtime/lean.lua --- Arc 8 Stage 3b: the Lean 4 language server.
--
-- Framing: `docs/lean4-mode-framing.md` Q#LN7 (lake serve + probe +
-- fallback latch), Q#LN8 (Lake-aware outermost root), Q#LN16
-- (waitForDiagnostics). Stage 1 shipped the grammar, mode, comment
-- strings and pair set; Stage 3a shipped the notification/response
-- seams and `pmacs.fs.canonicalize` this file consumes.
--
-- Loaded after `lsp.lua`, which owns `pmacs.lsp.config` and the drain.

local M = {}

-- Q#LN8 — the Lake-aware root -----------------------------------------
--
-- `pmacs.project.detect` cannot express this rule. It is innermost-wins
-- by construction, and a Lake package's `lean-toolchain` sits at the
-- OUTERMOST level: a file under `<pkg>/.lake/packages/dep/Foo.lean`
-- belongs to `<pkg>`'s server, not to `dep`'s, because `lake serve` is
-- bound to one package and analyzes its dependencies from inside it.
-- Inverting `detect` globally would change Rust/Go/Node roots for every
-- user, so the rule lives here as a function-valued `config.root` —
-- the generalization Stage 2 (#161) added for exactly this.

-- The marker test, and the two ways to get it wrong.
--
-- `pmacs.fs.stat` is UNUSABLE here: it returns an awaitable handle
-- (`fs.lua`), and this runs synchronously inside `ensure_server` <-
-- `attach_buffer` <- the `buffer.after-load` hook, where there is no
-- coroutine to await on. The Lua stdlib's `io.open` is the only
-- synchronous existence check available.
--
-- But `io.open` alone is wrong in BOTH directions:
--   * it SUCCEEDS on a directory (probed), so a truthiness test would
--     accept a `lean-toolchain` directory as a marker; and
--   * requiring a non-nil read rejects an EMPTY `lean-toolchain`, which
--     is a legitimate marker — `locate-dominating-file` semantics are
--     existence, not content.
-- The discriminator is `read`'s SECOND return (probed on LuaJIT 2.1):
--   file with content -> "l", no error      -> marker
--   empty file        -> nil, NO error      -> marker
--   directory         -> nil, "Is a directory" -> decline
--   missing           -> io.open returns nil   -> decline
-- so: decline only on a non-nil `err`. This needs no per-platform
-- re-probe, because both directory behaviors are declines — a platform
-- whose `fopen` refuses directories fails at `io.open` instead. There
-- is no platform where a directory both opens and yields a byte.
local function has_toolchain(dir)
  local f = io.open(dir .. "/lean-toolchain", "r")
  if not f then return false end
  local _, err = f:read(1)
  f:close()
  return err == nil
end

local function parent_of(dir)
  local up = dir:match("^(.*)/[^/]+$")
  if up == nil or up == dir or up == "" then return nil end
  return up
end

-- The walk stops at `pmacs.project.search_boundary()`. Not politeness:
-- `detect_project_within` (`src/project.rs`) exists precisely so a
-- stray marker above a temp fixture cannot leak into detection, and a
-- Lua walk that ignored the boundary would break that contract — and
-- make acceptance 23's outermost assertion non-hermetic against any
-- `lean-toolchain` sitting above the test's tempdir.
local function within_boundary(dir, boundary)
  if not boundary then return true end
  return dir == boundary or dir:sub(1, #boundary + 1) == boundary .. "/"
end

-- Returns the OUTERMOST ancestor holding a `lean-toolchain`, or nil to
-- decline (which falls through to `pmacs.project.detect`, then the
-- file's own directory).
--
-- **The result is canonical, and must be.** A configured root — which
-- this is — reaches `file_uri_for` verbatim and that URI is the
-- server-affinity key (#161). `pmacs.editor.file_path()` collapses `.`
-- and `..` lexically but leaves symlinks intact, so one package opened
-- through a symlink and through its real path would otherwise spawn two
-- `lake serve` processes. Canonicalizing ONCE up front is enough:
-- every ancestor of a canonical path is itself canonical, since the
-- walk only strips trailing components.
--
-- If canonicalization fails (deleted file, broken symlink) the resolver
-- declines rather than returning a path it cannot vouch for.
function M.root_for(path)
  if type(path) ~= "string" then return nil end
  local dir = path:match("^(.*)/[^/]*$")
  if not dir then return nil end
  dir = pmacs.fs.canonicalize(dir)
  if not dir then return nil end
  local boundary
  local ok, b = pcall(pmacs.project.search_boundary)
  if ok then boundary = b end
  -- The boundary is canonicalized at set time (`set_search_boundary`),
  -- so comparing it against a canonical `dir` is apples to apples.
  local outermost = nil
  local cur = dir
  while cur and within_boundary(cur, boundary) do
    if has_toolchain(cur) then outermost = cur end
    cur = parent_of(cur)
  end
  return outermost
end

-- Q#LN7 — `lake serve`, with a lazy probe and a one-shot latch --------
--
-- `pmacs.lsp.config.lean4` is declarative and must stay cheap: spawning
-- a process at startup for every user, Lean-using or not, is the cost
-- rev 1 refused. So no probe runs here — it runs on the first `.lean`
-- attach, below.
pmacs.lsp.config.lean4 = pmacs.lsp.config.lean4 or {
  command = "lake",
  args = { "serve" },
  root = M.root_for,
  -- No `init_options`: `hasWidgets?` defaults to false, which is the
  -- correct posture for a client reading plain goals out of standard
  -- messages rather than driving the `$/lean/rpc/*` widget stack.
}

-- Session state. The latch is one-shot and never re-arms: a user whose
-- toolchain is broken sees one fallback attempt, not a loop.
local probe = {
  started = false,   -- the `lake --version` probe has been spawned
  latched = false,   -- the fallback has fired (or been ruled out)
  proc = nil,        -- process id of the running probe
  buf = "",          -- accumulated probe stdout
  watching = nil,    -- sid we are waiting to see fail before initialize
  saw_initialized = false,
}

local function report(msg)
  -- COHERENCE §1.2: background work must leave an attributed trace.
  -- `pmacs.editor.set_status` is the channel that EXISTS; `pmacs.error`
  -- is referenced by fifteen call sites and defined nowhere in
  -- production, so it rides along rather than standing alone.
  pcall(pmacs.editor.set_status, msg)
  if pmacs.error then pcall(pmacs.error, msg) end
end

-- `lake serve` below 3.1.0 starts a server that cannot answer, which is
-- worse than failing: `lean4-mode` probes for exactly this and falls
-- back to `lean --server`. Parses the leading `x.y` of a version line.
-- State kind for `sid`, or nil if the manager has forgotten it.
local function server_state_kind(sid)
  local skey = tostring(sid)
  local ok, rows = pcall(pmacs.lsp.list)
  if not ok or not rows then return nil end
  for _, info in ipairs(rows) do
    if tostring(info.id) == skey then
      return info.state and info.state.kind
    end
  end
  return nil
end

local function version_below_3_1(text)
  local major, minor = text:match("(%d+)%.(%d+)")
  if not major then return false end
  major, minor = tonumber(major), tonumber(minor)
  if major < 3 then return true end
  return major == 3 and minor < 1
end

-- What the latch falls back TO. A table rather than a literal so the
-- acceptance suite can point it at a stand-in server and drive the real
-- latch path end to end, instead of asserting on a config mutation that
-- proves nothing about whether a server ever starts.
M.fallback = { command = "lean", args = { "--server" } }

-- Swap `command`/`args` ONLY. A wholesale table replacement would
-- silently discard a user's `env` / `settings` / `init_options` / `root`
-- from `init.lua` at exactly the moment they are least likely to notice.
--
-- The only guard is idempotence — already-the-fallback means nothing to
-- do. It deliberately does NOT refuse when the command is user-supplied:
-- the latch fires only when the configured Lean server actually failed
-- to start, and one visible fallback attempt beats leaving the user with
-- no server at all. `probe.latched` is what keeps it to exactly one.
local function swap_to_fallback()
  local cfg = pmacs.lsp.config.lean4
  if not cfg then return false end
  if cfg.command == M.fallback.command then return false end
  cfg.command = M.fallback.command
  cfg.args = M.fallback.args
  return true
end

-- Fire the fallback: stop the failing server FIRST, then swap, then let
-- the next attach spawn afresh.
--
-- Stopping first is load-bearing, not defensive. The spec default is
-- `LspRestartPolicy::OnCrash`, the termination handler never consults
-- the exit code, and `maybe_restart` has no attempt ceiling — so a
-- broken `lake` respawns forever on a backoff, underneath the latch,
-- producing a loop the latch cannot see the end of. `pmacs.lsp.stop`
-- sets `restart = Never` on the way out, which is what disarms it. The
-- fallback is therefore a FRESH server, not a restart of the old one.
local try_reattach

local function fire_latch(sid, why)
  if probe.latched then return end
  probe.latched = true
  probe.watching = nil
  -- **Only stop a server that is not ALREADY terminal**, and this is
  -- load-bearing rather than tidy. `LspManager::stop` on a crashed
  -- client takes its not-initialized branch: it terminates the
  -- (already-dead) process and sets `ShuttingDown { .. None }`, with the
  -- comment "the next exit observation cleans up" — but the exit was
  -- already observed, which is what made it `Crashed`. No further event
  -- arrives, so the client stays in `ShuttingDown` forever:
  -- `server_is_live` counts it as LIVE (neither crashed nor stopped) so
  -- `attach_buffer` never rebuilds, and `LspManager::forget` refuses it
  -- for not being terminal. Stopping a dead server is what makes it
  -- un-replaceable. Recorded as a substrate deferral in the framing §6.
  if sid then
    local kind = server_state_kind(sid)
    if kind and kind ~= "crashed" and kind ~= "stopped" then
      pcall(pmacs.lsp.stop, sid)
    end
  end
  if not swap_to_fallback() then
    report("LSP: lean4 " .. why)
    return
  end
  report("LSP: lean4 " .. why .. "; falling back to `"
    .. tostring(M.fallback.command) .. "`")
  -- **Spawn the replacement and re-point the buffer at it.** Stopping
  -- and rewriting the config is not a fallback on its own: nothing
  -- re-fires an attach on a config change, and `attach_buffer`
  -- early-returns for a live attachment, so without this the buffer
  -- stays bound to the server we just stopped and the user is left with
  -- a config edit and no language server. Round 1 shipped exactly that,
  -- with an acceptance test that asserted every server was terminal —
  -- i.e. that pinned the absence of the fallback it claimed to check.
  --
  -- **Retried on the tick, not done inline**, and that is not caution:
  -- `pmacs.lsp.stop` sends shutdown+exit and the state becomes
  -- `shutting-down`, which `server_is_live` counts as LIVE. So an
  -- immediate `_attach_buffer` early-returns the stale record and the
  -- swap has no effect — the exact silent no-op this whole path exists
  -- to avoid. Retrying until the old server actually reaches a terminal
  -- state is what makes the rebuild happen.
  probe.reattach_from = sid and tostring(sid) or false
  try_reattach()
end

-- Returns true once the active Lean buffer is attached to a server that
-- is not the one the latch stopped.
function try_reattach()
  if probe.reattach_from == nil then return true end
  local ok, rec = pcall(pmacs.lsp._attach_buffer)
  if not ok or not rec then return false end
  if probe.reattach_from and tostring(rec.server) == probe.reattach_from then
    return false
  end
  probe.reattach_from = nil
  return true
end

local function drain_probe()
  if not probe.proc then return end
  local ok, evs = pcall(pmacs.process.events_take, probe.proc)
  if not ok or not evs then return end
  for _, ev in ipairs(evs) do
    if ev.kind == "stdout" or ev.kind == "stderr" then
      probe.buf = probe.buf .. tostring(ev.bytes)
    elseif ev.kind == "exited" or ev.kind == "signaled"
        or ev.kind == "crashed" then
      local proc = probe.proc
      probe.proc = nil
      pcall(pmacs.process.forget, proc)
      -- A non-zero exit is NOT a fallback trigger on its own. §2.9: elan
      -- shims make `lake --version` exit non-zero with "no default
      -- toolchain configured" on a machine where `lake serve` may still
      -- be the right command — the server-failure latch covers that
      -- case, and covers it better. The probe answers only the ONE
      -- question failure detection would otherwise answer slowly: an
      -- old-but-working lake that starts a useless server.
      if ev.kind == "exited" and ev.code == 0
          and version_below_3_1(probe.buf) then
        fire_latch(probe.watching, "lake is older than 3.1.0")
      end
    end
  end
end

-- The probe cannot gate the first attach. There is no blocking process
-- run (§2.9): `spawn` + `events_take` off a tick is the only shape
-- available, so the verdict arrives AFTER `ensure_server` has already
-- had to decide. Hence the optimistic `lake serve` spawn, with the
-- probe and the latch correcting it.
local function start_probe(root)
  if probe.started then return end
  probe.started = true
  local cfg = pmacs.lsp.config.lean4
  if not cfg or not cfg.command then return end
  -- Probe the binary we would actually run, not the literal string
  -- "lake": a user pointing `command` at a wrapper or an absolute path
  -- should have THAT probed, and a hardcoded name would silently probe
  -- something else (or nothing).
  local spec = {
    -- COHERENCE §9: `ProcessSpec.label` is the only identity a process
    -- carries, and it is what `pmacs.process.list` renders. A user
    -- wondering why their editor touched `lake` finds an owner here.
    label = "lean:lake-version-probe",
    command = cfg.command,
    args = { "--version" },
    stdin = "null",
  }
  if root then spec.cwd = root end
  local ok, proc = pcall(pmacs.process.spawn, spec)
  if ok then probe.proc = proc end
  -- A probe that cannot even spawn says nothing the latch will not say
  -- more reliably a moment later, so it is not reported here.
end

-- How the latch observes server failure.
--
-- There is no event for "died before initialize" — the drain ignores
-- state events. So this polls `pmacs.lsp.list()` on the
-- `process.after-tick` cadence and treats a terminal state reached
-- WITHOUT an intervening `initialized` as the trigger. Watching stops
-- as soon as the server initializes, so an ordinary later crash (a real
-- server dying on a real error) does not silently rewrite the command.
local function poll_latch()
  local sid = probe.watching
  if not sid or probe.latched then return end
  local skey = tostring(sid)
  local ok, rows = pcall(pmacs.lsp.list)
  if not ok or not rows then return end
  for _, info in ipairs(rows) do
    if tostring(info.id) == skey then
      local kind = info.state and info.state.kind
      if kind == "initialized" then
        probe.saw_initialized = true
        probe.watching = nil
        return
      end
      if kind == "crashed" or kind == "stopped" then
        fire_latch(sid, "`lake serve` failed to start")
      end
      return
    end
  end
  -- Gone from the manager entirely without ever initializing.
  fire_latch(nil, "`lake serve` failed to start")
end

-- Q#LN16 — `textDocument/waitForDiagnostics` --------------------------
--
-- A plain request: no position, so Q#LN12's `outbound_position` concern
-- does not apply. Resolves when the server has finished elaborating.
-- Awaited through Stage 3a's response seam.
--
-- **`version` is required, not optional.** Lean's
-- `WaitForDiagnosticsParams` is `{ uri, version }` (v4.9.0,
-- `src/Lean/Data/Lsp/Extra.lean`), and the request is how the client
-- says *which* revision of the document it wants elaboration for.
-- Sending only `uri` is a malformed request against a real server; it
-- happened to look fine here because the fake server echoes any
-- payload. Callers pass the attachment's current `version`.
--
-- `fn(err)` is called with nil on success. Registering the one-shot
-- requires the server to have an attached buffer — see the note on
-- `pmacs.lsp.on_response`; every caller here comes from an attachment.
function M.wait_for_diagnostics(sid, uri, version, fn)
  local ok, rid = pcall(pmacs.lsp.send_request, sid,
    "textDocument/waitForDiagnostics", { uri = uri, version = version })
  if not ok then
    if fn then pcall(fn, tostring(rid)) end
    return nil
  end
  if fn then
    pmacs.lsp.on_response(sid, rid, function(_, err)
      fn(err and err.message or nil)
    end)
  end
  return rid
end

pmacs.command.define {
  name = "lean.wait-for-diagnostics",
  description = "Wait for the Lean server to finish elaborating this file",
  fn = function()
    local rec = pmacs.lsp.active_attachment()
    if not rec or rec.language ~= "lean4" then
      pmacs.editor.set_status("lean: no Lean server for this buffer")
      return
    end
    pmacs.editor.set_status("lean: elaborating…")
    M.wait_for_diagnostics(rec.server, rec.uri, rec.version, function(err)
      if err then
        pmacs.editor.set_status("lean: " .. tostring(err))
      else
        pmacs.editor.set_status("lean: elaboration complete")
      end
    end)
  end,
}

-- `$/lean/fileProgress` — the elaboration-in-flight signal. Stage 5's
-- goal view reads it to distinguish "no goals" from "not done yet";
-- here it is recorded so that consumer has something to read and so the
-- notification seam has its first production subscriber.
M.file_progress = {}

pmacs.lsp.on_notification("$/lean/fileProgress", function(_, params)
  local uri = params and params.textDocument and params.textDocument.uri
  if type(uri) ~= "string" then return end
  M.file_progress[uri] = params.processing or {}
end)

-- Wiring --------------------------------------------------------------

-- Runs after `lsp.lua`'s own `buffer.after-load` subscription.
--
-- **Keyed on the buffer's LANGUAGE, not on an attachment existing.**
-- Round 1 keyed on `active_attachment()` and returned early when it was
-- nil — which silently excluded the single most likely real-world
-- failure: `lake` not installed. `ensure_server` pcalls the spawn and
-- returns nil on ENOENT, so `attach_buffer` produces no record at all,
-- so the probe never started and the latch never armed. The case the
-- fallback exists for was the one case it could not see.
pmacs.hook.add("buffer.after-load", function()
  local buf = pmacs.window.buffer()
  if not buf then return end
  local ok_lang, lang = pcall(pmacs.lsp.buffer_language, buf)
  if not ok_lang or lang ~= "lean4" then return end

  if not probe.started then
    local path = pmacs.editor.file_path()
    start_probe(path and M.root_for(path) or nil)
  end

  local rec = pmacs.lsp.active_attachment()
  if rec and rec.language == "lean4" then
    -- Watch only the FIRST Lean server: the latch is per session.
    if not probe.latched and not probe.saw_initialized
        and probe.watching == nil then
      probe.watching = rec.server
    end
    return
  end

  -- No attachment for a Lean buffer means `ensure_server` could not
  -- spawn at all — a synchronous ENOENT, already swallowed upstream.
  -- That is not something to wait for; it is the failure itself, and
  -- the only place it is still observable.
  if not probe.latched then
    fire_latch(nil, "`" .. tostring(
      pmacs.lsp.config.lean4 and pmacs.lsp.config.lean4.command)
      .. "` could not be started")
  end
end)

pmacs.hook.add("process.after-tick", function()
  drain_probe()
  poll_latch()
  -- Keep trying until the stopped server is really gone; see the note in
  -- `fire_latch`.
  if probe.reattach_from ~= nil then try_reattach() end
end)

-- Test seam: acceptance drives the latch deterministically rather than
-- waiting on real process timing. Not part of the public surface.
M._probe = probe
M._fire_latch = fire_latch
M._version_below_3_1 = version_below_3_1

pmacs.lean = M
