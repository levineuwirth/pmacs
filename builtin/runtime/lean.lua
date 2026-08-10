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
  out = "",          -- accumulated probe stdout
  buf_key = nil,     -- tostring() of the buffer that started this
  watching = nil,    -- sid still being polled for die-before-initialize
  primary = nil,     -- sid the probe's verdict applies to; NOT cleared
                     -- when the server initializes, because a late
                     -- version verdict still has to retire it
  armed = false,     -- the target buffer + primary have been captured
  repaired = {},     -- buffer key -> repair attempted (at most once)
  repair_attempts = 0, -- COUNT of attach attempts, not distinct buffers:
                     -- table cardinality cannot tell "once per buffer"
                     -- from "every tick for one buffer"
  fallback_installed = false,
  fallback_watches = {}, -- sid key -> sid, each polled die-before-init
  fallback_done = {}, -- sid key -> initialized or terminally handled
  saw_initialized = false,
}

-- The command as configured, for status text. Hardcoding "lake serve"
-- was untruthful the moment the failure latch became command-agnostic:
-- a user whose `my-lean-wrapper` failed was told `lake serve` did.
local function configured_command()
  local cfg = pmacs.lsp.config.lean4
  local cmd = cfg and cfg.command
  if not cmd then return "the Lean server" end
  local args = cfg.args or {}
  if #args > 0 then
    return "`" .. tostring(cmd) .. " " .. table.concat(args, " ") .. "`"
  end
  return "`" .. tostring(cmd) .. "`"
end

-- The fallback command, for status text.
local function fallback_name()
  local args = M._fallback.args or {}
  if #args > 0 then
    return "`" .. tostring(M._fallback.command) .. " "
      .. table.concat(args, " ") .. "`"
  end
  return "`" .. tostring(M._fallback.command) .. "`"
end

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
-- State kind for the server whose `tostring(id)` is `skey`, or nil if
-- the manager has forgotten it (which is itself a terminal answer).
local function server_state_kind_for_key(skey)
  local ok, rows = pcall(pmacs.lsp.list)
  if not ok or not rows then return nil end
  for _, info in ipairs(rows) do
    if tostring(info.id) == skey then
      return info.state and info.state.kind
    end
  end
  return nil
end

local function server_state_kind(sid)
  return server_state_kind_for_key(tostring(sid))
end

local function version_below_3_1(text)
  local major, minor = text:match("(%d+)%.(%d+)")
  if not major then return false end
  major, minor = tonumber(major), tonumber(minor)
  if major < 3 then return true end
  return major == 3 and minor < 1
end

-- What the latch falls back TO.
--
-- **Underscored: a test seam, not supported user configuration.** It is
-- a table only so the acceptance suite can point it at a stand-in server
-- and drive the real latch path end to end, instead of asserting on a
-- config mutation that proves nothing about whether a server ever
-- starts. Presenting it as public config would owe framing,
-- documentation, validation and mutation semantics that nothing here
-- provides; users configure Lean through `pmacs.lsp.config.lean4`.
M._fallback = { command = "lean", args = { "--server" } }

local function same_args(a, b)
  a, b = a or {}, b or {}
  if #a ~= #b then return false end
  for i = 1, #a do
    if a[i] ~= b[i] then return false end
  end
  return true
end

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
  -- Idempotence compares command AND args: the same command with
  -- different arguments is not "already applied", and treating it as
  -- such would silently skip a swap that still needed to happen.
  if cfg.command == M._fallback.command
      and same_args(cfg.args, M._fallback.args) then
    return false
  end
  cfg.command = M._fallback.command
  cfg.args = M._fallback.args
  return true
end

-- Retire the failed server, swap the command, then rebuild the
-- attachment on the buffer that started this.
-- Retire `sid` so it cannot come back. **Which call to use depends on
-- the state, and using the wrong one is worse than doing nothing:**
--
--   * TERMINAL (`crashed` / `stopped`) -> `forget`. It requires a
--     terminal state and removes the client outright, which also drops
--     the `next_restart_at` the crash scheduled. `stop` here would take
--     its not-initialized branch and set `ShuttingDown { .. None }` on
--     the premise that "the next exit observation cleans up" — but the
--     exit already happened, which is what made it `Crashed`. No
--     further event arrives, so it sits in `ShuttingDown` forever:
--     `server_is_live` reads that as LIVE so `attach_buffer` never
--     rebuilds, and `forget` then refuses it for not being terminal.
--   * NON-TERMINAL -> `stop`. `forget` rejects it, and `stop` disables
--     restart and drives the polite shutdown.
--
-- Round 1 skipped the call entirely for terminal servers. That avoided
-- the corruption but left `next_restart_at` armed, so the crashed
-- primary respawned 500ms later and kept respawning underneath the
-- live fallback — invisible to a test that stopped ticking first.
local function retire_server(sid)
  local kind = server_state_kind(sid)
  if kind == nil then return end
  if kind == "crashed" or kind == "stopped" then
    pcall(pmacs.lsp.forget, sid)
  else
    pcall(pmacs.lsp.stop, sid)
  end
end

-- Retire EVERY Lean server, not just the one that failed.
--
-- `pmacs.lsp.config.lean4` is a single global entry, so swapping its
-- command invalidates every server spawned from the old one — and
-- Q#LN15 gives one server per project root, so there can be several.
-- Retiring only the server that happened to fail left the others live
-- and every buffer attached to them stranded on a command the config no
-- longer names.
-- Only servers the config-driven path itself produced. A server's label,
-- language, command, and root are all caller-supplied public values; none
-- is an ownership discriminator. `lsp.lua` records the successful spawn
-- in a private origin table, which is the fact this lifecycle may act on.
local function is_derived_server(sid)
  local ok, owned = pcall(pmacs.lsp._is_default_server, sid, "lean4")
  return ok and owned == true
end

local function retire_derived_lean_servers()
  local ok, rows = pcall(pmacs.lsp.list)
  if not ok or not rows then return end
  local ids = {}
  for _, info in ipairs(rows) do
    if is_derived_server(info.id) then ids[#ids + 1] = info.id end
  end
  for _, id in ipairs(ids) do
    -- These ids predate the fallback spawn. Mark them handled before
    -- retirement so the discovery poll cannot mistake their terminal
    -- state for a fallback that failed to initialize.
    probe.fallback_done[tostring(id)] = true
    retire_server(id)
  end
end

local function watch_fallback_server(sid)
  if not sid or not is_derived_server(sid) then return end
  local key = tostring(sid)
  if probe.fallback_done[key] then return end
  probe.fallback_watches[key] = sid
end

-- Rebuild the ACTIVE buffer's attachment if it is Lean and stale.
--
-- `_attach_buffer` is an active-buffer-only seam, so a global config
-- swap cannot be applied to every open buffer at once. It is applied
-- lazily instead: whenever a Lean buffer becomes the active one, if its
-- record points at a server that is gone or terminal, it is rebuilt.
--
-- **At most one attempt per buffer.** Without that bound a fallback
-- that also fails to spawn would retry every tick forever with nothing
-- reported — the round-2 defect, which a general repair loop would
-- otherwise reintroduce for every buffer instead of just one.
--
-- A `shutting-down` server is deliberately NOT treated as stale: it is
-- still live by `server_is_live`'s reckoning, so `attach_buffer` would
-- early-return the stale record and burn this buffer's single attempt
-- on a no-op. Skipping leaves the attempt for a later tick, once the
-- retirement has actually landed.
local function repair_active_if_stale()
  -- **`fallback_installed`, not `latched`.** When the swap does not
  -- happen — the config already names the fallback, or it vanished
  -- before an asynchronous verdict landed — `fire_latch` returns early
  -- but `latched` stays true. Gating repair on `latched` then retried
  -- the UNCHANGED configuration and reported the result as a fallback
  -- failure, which is both a second pointless spawn and a misleading
  -- message. Repair exists to apply a swap; no swap, nothing to apply.
  if not probe.fallback_installed then return end
  local buf = pmacs.window.buffer()
  if not buf then return end
  local key = tostring(buf)
  if probe.repaired[key] then return end
  local ok_lang, lang = pcall(pmacs.lsp.buffer_language, buf)
  if not ok_lang or lang ~= "lean4" then return end

  local rec = pmacs.lsp.active_attachment()
  local stale
  if not rec then
    stale = true
  else
    local kind = server_state_kind(rec.server)
    stale = (kind == nil or kind == "crashed" or kind == "stopped")
  end
  if not stale then return end

  probe.repaired[key] = true
  probe.repair_attempts = probe.repair_attempts + 1
  local ok, fresh = pcall(pmacs.lsp._attach_buffer)
  if not ok or not fresh then
    report("LSP: lean4 fallback " .. fallback_name()
      .. " did not start either")
    return
  end
  -- **A successful SPAWN is not a successful START.** The once-per-
  -- buffer bound stops `_attach_buffer` being called again, but it says
  -- nothing about the server it produced. Arm this id immediately; the
  -- poll below also discovers servers created through lsp.lua's own
  -- after-load and command paths.
  watch_fallback_server(fresh.server)
end

-- Every fallback server gets its own die-before-initialize poll. A scalar
-- watch cannot cover Q#LN15's simultaneous per-root servers, and a server
-- may be created by lsp.lua's after-load or command path without passing
-- through `repair_active_if_stale`. Discovery from the private ownership
-- table closes both holes.
local function poll_fallbacks()
  if not probe.fallback_installed then return end
  local ok, rows = pcall(pmacs.lsp.list)
  if not ok or not rows then return end

  local by_key = {}
  for _, info in ipairs(rows) do
    local key = tostring(info.id)
    by_key[key] = info
    if not probe.fallback_done[key] and is_derived_server(info.id) then
      probe.fallback_watches[key] = info.id
    end
  end

  for key, sid in pairs(probe.fallback_watches) do
    local info = by_key[key]
    local kind = info and info.state and info.state.kind
    if kind == "initialized" then
      probe.fallback_watches[key] = nil
      probe.fallback_done[key] = true
    elseif info == nil or kind == "crashed" or kind == "stopped" then
      probe.fallback_watches[key] = nil
      probe.fallback_done[key] = true
      if info ~= nil then retire_server(sid) end
      report("LSP: lean4 fallback " .. fallback_name()
        .. " started but did not stay up")
    end
  end
end

local function fire_latch(sid, why)
  if probe.latched then return end
  probe.latched = true
  probe.watching = nil
  if not swap_to_fallback() then
    report("LSP: lean4 " .. why)
    -- No shared config changed, so only the server whose failure
    -- triggered this verdict is invalid. Sweeping every root here stops
    -- healthy instances of a root-sensitive command for no reason.
    if sid and is_derived_server(sid) then retire_server(sid) end
    return
  end
  retire_derived_lean_servers()
  probe.fallback_installed = true
  report("LSP: lean4 " .. why .. "; falling back to " .. fallback_name())
  -- Repair what is in front of the user now; everything else is
  -- repaired lazily as it becomes active (see `repair_active_if_stale`).
  repair_active_if_stale()
end

local function drain_probe()
  if not probe.proc then return end
  local ok, evs = pcall(pmacs.process.events_take, probe.proc)
  if not ok or not evs then return end
  for _, ev in ipairs(evs) do
    if ev.kind == "stdout" or ev.kind == "stderr" then
      probe.out = probe.out .. tostring(ev.bytes)
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
      -- **`probe.primary`, NOT `probe.watching`.** `watching` is
      -- failure-polling state and is cleared the moment the server
      -- initializes. A slow `--version` that lands after a successful
      -- initialize would then arrive with nil, and `fire_latch(nil)`
      -- retires nothing: `_attach_buffer` finds the still-live primary
      -- attachment, early-returns it, and the retry calls that success.
      -- Status and config would say "fell back" while the buffer stayed
      -- on the old server — the same silent no-op as round 1, reached
      -- through a different event ordering. Initializing must stop the
      -- failure poll, not erase the server the verdict has to retire.
      if ev.kind == "exited" and ev.code == 0
          and version_below_3_1(probe.out) then
        fire_latch(probe.primary, "lake is older than 3.1.0")
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
  -- **Only probe something actually named `lake`.** `version_below_3_1`
  -- parses the first `x.y` it finds anywhere in the output, which is a
  -- rule about LAKE's output contract and nothing else. Run against a
  -- user's wrapper it is a category error: a working `my-lean-wrapper`
  -- reporting "wrapper 1.0" would be replaced despite its server having
  -- initialized fine. The FAILURE latch stays command-agnostic — that
  -- one keys on the server actually not starting, which is true of any
  -- command — but the version rule only applies where its contract
  -- holds.
  local base = cfg.command:match("([^/]+)$") or cfg.command
  if base ~= "lake" then return end
  -- Probe the binary we would actually run, not the literal string
  -- "lake": a user pointing `command` at an absolute path to lake should
  -- have THAT probed, not whatever `lake` resolves to on PATH.
  local spec = {
    -- COHERENCE §9: `ProcessSpec.label` identifies the process, and it
    -- is what `pmacs.process.list` renders alongside the purpose. A user
    -- wondering why their editor touched `lake` finds it here.
    label = "lean:lake-version-probe",
    -- Worker identity Stage 1: the label was carrying both jobs — the
    -- identity AND the explanation — which is the conflation the purpose
    -- field exists to undo. The label stays a key; this is the sentence.
    purpose = "checking the Lean toolchain version before starting a server",
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
        -- Stop polling for failure; `probe.primary` deliberately
        -- survives, because a later version verdict still needs it.
        probe.saw_initialized = true
        probe.watching = nil
        return
      end
      if kind == "crashed" or kind == "stopped" then
        fire_latch(sid, configured_command() .. " failed to start")
      end
      return
    end
  end
  -- Gone from the manager entirely without ever initializing.
  fire_latch(nil, configured_command() .. " failed to start")
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

local function when_server_ready(sid, fn)
  local function state_kind()
    local ok, state = pcall(pmacs.lsp.status, sid)
    if not ok or not state then return nil end
    return state.kind
  end

  local kind = state_kind()
  if kind == "initialized" then
    fn(nil)
    return
  end
  if kind == nil or kind == "crashed" or kind == "stopped" then
    fn("server did not initialize")
    return
  end

  -- A command may have just healed a dead attachment, in which case the
  -- replacement is still starting. Requests are not queued before
  -- initialize, so issue this one after the lifecycle reaches ready
  -- rather than replacing the attachment and immediately failing on it.
  pmacs.async(function()
    for _ = 1, 300 do
      pmacs.async.yield_to_next_tick()
      kind = state_kind()
      if kind == "initialized" then
        fn(nil)
        return
      end
      if kind == nil or kind == "crashed" or kind == "stopped" then
        fn("server did not initialize")
        return
      end
    end
    fn("server initialization timed out")
  end)
end

pmacs.command.define {
  name = "lean.wait-for-diagnostics",
  description = "Wait for the Lean server to finish elaborating this file",
  fn = function()
    local rec = pmacs.lsp._attachment_for_command()
    if not rec or rec.language ~= "lean4" then
      pmacs.editor.set_status("lean: no Lean server for this buffer")
      return
    end
    pmacs.editor.set_status("lean: elaborating…")
    when_server_ready(rec.server, function(init_err)
      if init_err then
        pmacs.editor.set_status("lean: " .. tostring(init_err))
        return
      end
      M.wait_for_diagnostics(rec.server, rec.uri, rec.version, function(err)
        if err then
          pmacs.editor.set_status("lean: " .. tostring(err))
        else
          pmacs.editor.set_status("lean: elaboration complete")
        end
      end)
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

  local rec = pmacs.lsp.active_attachment()
  if rec and rec.language == "lean4" then
    -- A matching-root server supplied by the user may be adopted by
    -- `ensure_server`. Its lifecycle is not evidence about the
    -- config-driven command, and neither the version probe nor fallback
    -- latch may mutate config because that foreign server changed state.
    if not is_derived_server(rec.server) then return end
    if not probe.started then
      local path = pmacs.editor.file_path()
      start_probe(path and M.root_for(path) or nil)
    end
    -- **Arm ONCE, capturing buffer and server together.** Setting
    -- `buf_key` on every Lean load meant a second Lean buffer opened
    -- before the verdict silently became the rebuild target while the
    -- latch still watched the FIRST buffer's server — so the rebuild
    -- either repaired the wrong buffer or accepted the second buffer's
    -- unrelated live server as success, stranding the first. The pair
    -- (target buffer, primary server) is one fact and is captured as
    -- one.
    if not probe.armed and not probe.latched and not probe.saw_initialized then
      probe.armed = true
      probe.buf_key = tostring(buf)
      probe.primary = rec.server
      probe.watching = rec.server
    end
    return
  end

  -- **Unconfigured is DISABLED, not failed.** A user who sets
  -- `pmacs.lsp.config.lean4 = nil`, or clears its `command`, has turned
  -- the Lean server off; reporting that "nil could not be started" is a
  -- false alarm, and latching would poison the session so a later
  -- configuration could never take effect. Only a CONFIGURED command
  -- that produced no attachment is a failure.
  local cfg = pmacs.lsp.config.lean4
  if not cfg or not cfg.command then return end
  if not probe.started then
    local path = pmacs.editor.file_path()
    start_probe(path and M.root_for(path) or nil)
  end

  -- No attachment for a Lean buffer with a configured command means
  -- `ensure_server` could not spawn at all — a synchronous ENOENT,
  -- already swallowed upstream. That is not something to wait for; it
  -- is the failure itself, and the only place it is still observable.
  if not probe.latched then
    -- No server was ever created, so there is no primary to retire —
    -- but the rebuild still needs a target buffer.
    if not probe.armed then
      probe.armed = true
      probe.buf_key = tostring(buf)
    end
    fire_latch(nil, configured_command() .. " could not be started")
  end
end)

-- A buffer switch is the moment a stale Lean buffer becomes visible, so
-- repair immediately rather than waiting for the next tick. lsp.lua's
-- own `after-switch` subscription re-pushes views but does NOT rebuild a
-- stale attachment, so nothing else covers this.
pmacs.hook.add("buffer.after-switch", function()
  repair_active_if_stale()
end)

pmacs.hook.add("process.after-tick", function()
  drain_probe()
  poll_latch()
  -- Repair the active buffer if the latch invalidated it. Cheap when
  -- there is nothing to do, and bounded to one attempt per buffer.
  repair_active_if_stale()
  poll_fallbacks()
end)

-- Test seam: acceptance drives the latch deterministically rather than
-- waiting on real process timing. Not part of the public surface.
M._probe = probe
M._fire_latch = fire_latch
M._version_below_3_1 = version_below_3_1

pmacs.lean = M
