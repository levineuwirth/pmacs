-- finder.lua --- the project-wide file finder (E4.2, the Ctrl-P moment).
--
-- `M-x project.find-file` (bound `C-x p f` in builtin/keymaps/default.lua)
-- prompts over every file under the project root, ranked by the
-- minibuffer's own fuzzy scorer with the recentf list at the top, and
-- opens the choice through `pmacs.buffer.find_or_open`.
--
-- The listing is a job and never a synchronous walk. A function source
-- runs on the main thread on every keystroke (`recompute_candidates`),
-- so the tree is enumerated ONCE per prompt off the main thread and the
-- source only ranks what has arrived. Two enumerators, tried in order:
--
--   1. `git ls-files -z --cached --others --exclude-standard`, when git
--      can be spawned and the root is inside a work tree. That is the
--      one way to honor `.gitignore` faithfully: the ignore language is
--      git's, and a reimplementation is how a finder ends up listing
--      `target/`. The child exits non-zero outside a repository, which
--      is the signal to fall back rather than an error to report.
--   2. `pmacs.fs.walk_tree(root, { supersede = "finder" })` otherwise:
--      every regular file under the root minus `.git/` itself, with no
--      ignore rules, because there is no `.gitignore` semantics without
--      git.
--
-- Both are CANCELLED when the prompt is cancelled and SUPERSEDED when a
-- new prompt opens before the last listing landed: the walk through its
-- supersede key and its handle's `cancel`, the git child through
-- `pmacs.process.terminate`. A generation counter is what makes the
-- supersede safe --- a late result from an older prompt is dropped, so
-- it can never refill a newer prompt over a different root.
--
-- Ranking is Rust's (`pmacs.minibuffer.rank`, the session's own
-- `fuzzy_score`), applied to the recent files and to the listing
-- separately and then merged with the recents first. The prompt is
-- opened `ranked = true` (E4.1) so that merge order IS the candidate
-- order; no score expresses "recently visited", so it has to be a
-- placement rather than a bonus.

pmacs.finder = pmacs.finder or {}

-- Mirrors `CANDIDATE_LIMIT` in src/minibuffer.rs: the session keeps at
-- most this many candidates, so returning more only costs the crossing.
local CANDIDATE_LIMIT = 1024

local PROMPT_HISTORY = "project-find-file"
local SUPERSEDE_KEY = "finder"

-- The one in-flight or landed listing. `generation` is bumped per
-- prompt; every asynchronous continuation compares against it first.
local state = {
  generation = 0,
  root = nil,
  prompt = nil,    -- the prompt string this generation opened with
  files = nil,     -- root-relative paths once the listing landed
  via = nil,       -- "git" | "walk" once landed
  walk = nil,      -- walk_tree handle while a walk is in flight
  proc = nil,      -- process handle while git is in flight
  listing = false,
}

local function join(root, rel)
  if root:sub(-1) == "/" then return root .. rel end
  return root .. "/" .. rel
end

local function dirname(path)
  local dir = path:match("^(.*)/[^/]*$")
  if dir == nil then return nil end
  if dir == "" then return "/" end
  return dir
end

-- The root a prompt lists: the active file's project (marker walk), else
-- that file's own directory, else the daemon's working directory's
-- project or the directory itself, else ".". The same precedence
-- `compile.lua` and `project.search` resolve by, so the three project
-- surfaces agree about what "the project" is from a given buffer.
local function project_root()
  local buf = pmacs.window.buffer()
  if buf then
    local ok, path = pcall(function() return buf:path() end)
    if ok and path then
      local okd, proj = pcall(pmacs.project.detect, path)
      if okd and proj and proj.root then return proj.root end
      local dir = dirname(path)
      if dir then return dir end
    end
  end
  local cwd = "."
  local ok, id = pcall(pmacs.instance.identity)
  if ok and type(id) == "table" and type(id.working_directory) == "string" then
    cwd = id.working_directory
  end
  local okd, proj = pcall(pmacs.project.detect, cwd)
  if okd and proj and proj.root then return proj.root end
  return cwd
end

-- ---------------------------------------------------------------------
-- Cancellation and supersession
-- ---------------------------------------------------------------------

-- Forward declaration: the git pump is defined below.
local pump

-- Stop whatever the current generation still has in flight and RETIRE
-- the generation, so a result that had already been produced (a git
-- child that exited before the cancel, whose events are still queued
-- for the next tick) is dropped by the generation check rather than
-- delivered to a prompt that no longer exists.
local function stop_in_flight()
  local had = false
  if state.walk then
    had = true
    pcall(state.walk.cancel, state.walk)
    state.walk = nil
  end
  if state.proc then
    had = true
    local okr, raw = pcall(state.proc.raw, state.proc)
    if okr then pump[raw] = nil end
    pcall(pmacs.process.terminate, state.proc)
    pcall(pmacs.process.forget, state.proc)
    state.proc = nil
  end
  if had then state.generation = state.generation + 1 end
  state.listing = false
end

-- ---------------------------------------------------------------------
-- Delivery
-- ---------------------------------------------------------------------

-- Refresh the prompt only when it is still THIS generation's prompt: the
-- user may have cancelled and opened `M-x` since, and a refresh there
-- would recompute an unrelated session (harmlessly, but pointlessly).
local function refresh_prompt_if_ours()
  if pmacs.minibuffer.is_active() and pmacs.minibuffer.prompt() == state.prompt then
    pmacs.minibuffer.refresh()
  end
end

local function deliver(gen, files, via)
  if gen ~= state.generation then return end
  state.files = files
  state.via = via
  state.listing = false
  state.walk = nil
  state.proc = nil
  refresh_prompt_if_ours()
end

-- ---------------------------------------------------------------------
-- Enumerator 2: the walk
-- ---------------------------------------------------------------------

local function start_walk(gen, root)
  local ok, handle = pcall(pmacs.fs.walk_tree, root, { supersede = SUPERSEDE_KEY })
  if not ok then
    state.listing = false
    pmacs.editor.set_status("finder: " .. tostring(handle))
    return
  end
  state.walk = handle
  pmacs.async(function()
    local okw, entries = pcall(function() return handle:await() end)
    if gen ~= state.generation then return end
    if not okw then
      state.listing = false
      state.walk = nil
      pmacs.editor.set_status("finder: " .. tostring(entries))
      return
    end
    local files = {}
    for _, e in ipairs(entries) do
      local name = e.name
      if e.kind == "file" and name ~= ".git" and name:sub(1, 5) ~= ".git/" then
        files[#files + 1] = name
      end
    end
    table.sort(files)
    deliver(gen, files, "walk")
  end)
end

-- ---------------------------------------------------------------------
-- Enumerator 1: git ls-files
-- ---------------------------------------------------------------------

-- Per-process output accumulators, drained on `process.after-tick` the
-- way git.lua drains its own spawns. Keyed by the process's raw id.
pump = {}

pmacs.hook.add("process.after-tick", function()
  for raw, entry in pairs(pump) do
    for _, ev in ipairs(pmacs.process.events_take(entry.procid)) do
      local kind = ev.kind
      if kind == "stdout" then
        entry.out[#entry.out + 1] = ev.bytes
      elseif kind == "exited" or kind == "signaled" or kind == "crashed" then
        pump[raw] = nil
        pcall(pmacs.process.forget, entry.procid)
        entry.on_done(kind == "exited" and (ev.code or 0) or nil, table.concat(entry.out))
      end
    end
  end
end)

local function git_program()
  if pmacs.git and type(pmacs.git._program) == "string" then return pmacs.git._program end
  return "git"
end

local function git_enabled()
  local ok, value = pcall(pmacs.config.get, "git.enabled")
  if not ok then return true end
  return value ~= false
end

-- Try git; on any failure to spawn or a non-zero exit (not a repository,
-- no git on PATH), fall back to the walk for the same generation.
local function start_git_or_walk(gen, root)
  if not git_enabled() then
    start_walk(gen, root)
    return
  end
  local ok, proc = pcall(pmacs.process.spawn, {
    label = "finder",
    purpose = "list project files for the finder",
    command = git_program(),
    args = { "-C", root, "ls-files", "-z", "--cached", "--others", "--exclude-standard" },
    cwd = root,
    stdin = "null",
  })
  if not ok then
    start_walk(gen, root)
    return
  end
  state.proc = proc
  pump[proc:raw()] = {
    procid = proc,
    out = {},
    on_done = function(code, stdout)
      if gen ~= state.generation then return end
      state.proc = nil
      if code ~= 0 then
        start_walk(gen, root)
        return
      end
      local files = {}
      for name in stdout:gmatch("([^%z]+)") do
        files[#files + 1] = name
      end
      deliver(gen, files, "git")
    end,
  }
end

-- ---------------------------------------------------------------------
-- Ranking
-- ---------------------------------------------------------------------

-- The recentf list narrowed to files under `root`, as root-relative
-- paths in MRU order. `pmacs.recentf.list` is absolute paths.
local function recent_under(root)
  local out = {}
  if not (pmacs.recentf and pmacs.recentf.list) then return out end
  local ok, list = pcall(pmacs.recentf.list)
  if not ok or type(list) ~= "table" then return out end
  local prefix = root:sub(-1) == "/" and root or (root .. "/")
  for _, path in ipairs(list) do
    if type(path) == "string" and path:sub(1, #prefix) == prefix then
      out[#out + 1] = path:sub(#prefix + 1)
    end
  end
  return out
end

-- The prompt's source: recents that match the needle first, in MRU
-- order among equal scores (rank is stable on ties only lexically, so
-- the recents are ranked and then re-ordered by their MRU position),
-- then the listing's best matches, deduplicated against the recents.
local function candidates_for(needle)
  local root = state.root
  local recents = recent_under(root)
  local ranked_recent = pmacs.minibuffer.rank(needle, recents)
  local mru_index = {}
  for i, rel in ipairs(recents) do mru_index[rel] = mru_index[rel] or i end
  table.sort(ranked_recent, function(a, b) return mru_index[a] < mru_index[b] end)
  local out = {}
  local seen = {}
  for _, rel in ipairs(ranked_recent) do
    if not seen[rel] then
      seen[rel] = true
      out[#out + 1] = rel
    end
  end
  if not state.files then return out end
  local ranked = pmacs.minibuffer.rank(needle, state.files, CANDIDATE_LIMIT + #out)
  for _, rel in ipairs(ranked) do
    if #out >= CANDIDATE_LIMIT then break end
    if not seen[rel] then
      seen[rel] = true
      out[#out + 1] = rel
    end
  end
  return out
end

-- ---------------------------------------------------------------------
-- The command
-- ---------------------------------------------------------------------

function pmacs.finder.open(opts)
  opts = opts or {}
  -- A new prompt supersedes whatever the last one still had in flight:
  -- `minibuffer.read` replaces a live session without calling its
  -- `on_cancel`, so the previous generation's job has to be stopped
  -- here, not there.
  stop_in_flight()
  state.generation = state.generation + 1
  local gen = state.generation
  local root = opts.root or project_root()
  state.root = root
  state.files = nil
  state.via = nil
  state.listing = true
  state.prompt = string.format("Find file in %s: ", root)
  pmacs.minibuffer.read {
    prompt = state.prompt,
    history = PROMPT_HISTORY,
    ranked = true,
    source = candidates_for,
    on_accept = function(rel)
      if gen == state.generation then stop_in_flight() end
      if rel == nil or rel == "" then return end
      local path = rel:sub(1, 1) == "/" and rel or join(root, rel)
      local ok, err = pcall(pmacs.buffer.find_or_open, path)
      if not ok then
        pmacs.editor.set_status("finder: " .. tostring(err))
      end
    end,
    on_cancel = function()
      if gen == state.generation then stop_in_flight() end
    end,
  }
  start_git_or_walk(gen, root)
end

-- What a test or a user script can ask: which root, whether a listing
-- is still in flight, how many files landed and by which enumerator.
function pmacs.finder.status()
  return {
    generation = state.generation,
    root = state.root,
    listing = state.listing,
    files = state.files and #state.files or nil,
    via = state.via,
  }
end

pmacs.command.define {
  name = "project.find-file",
  description = "Find a file anywhere in the project by fuzzy name; recent files first.",
  fn = function() pmacs.finder.open() end,
}
