-- pmacs-magit/status.lua --- Git status fetching + parsing (T M8.6).
--
-- The magit-class layer that turns a Git repo into a section spec
-- ready for the M8.5 fold module. Runs `git` via
-- `pmacs.process.spawn`, parses stdout into structured sections,
-- assembles the canonical 5-section layout (working tree, staged,
-- recent commits, branches, stashes).
--
-- This module is split out so the parsers are unit-testable without
-- spawning git: each `parse_*` function takes a string and returns
-- a Lua table describing the parsed result. `fetch_sections`
-- orchestrates the four `git` invocations sequentially and assembles
-- a section spec the caller hands to the fold module.
--
-- Public surface:
--
--   run_git(args, opts)             -> { ok, stdout, stderr, exit_code }
--   parse_porcelain_v2(text)        -> { unstaged, staged, untracked, branch }
--   parse_log(text)                 -> array of { hash, subject }
--   parse_branches(text)            -> { current, all = [...] }
--   parse_stashes(text)             -> array of { ref, subject }
--   build_spec(parsed)              -> section spec table
--   fetch_sections(repo_root)       -> section spec table (calls run_git x4)
--
-- "Sequentially" rather than in parallel: in v0.1, four sequential
-- git invocations on a typical repo total ~50-100ms. Polling cadence
-- is 250 ms (see init.lua), so the worst-case latency to reflect an
-- external change is ~350 ms --- under the M8.6 acceptance bullet's
-- 500 ms budget. Parallel invocation is a v0.2 optimization that
-- requires futures-style joining (see `V0.2-PREREQUISITES.md SP-2`).

local M = {}

-- ---------------------------------------------------------------------------
-- run_git: spawn `git`, drain stdout/stderr, await exit
-- ---------------------------------------------------------------------------
--
-- Must be called inside a `pmacs.async` coroutine because it yields
-- via `pmacs.workers.sleep(...):await()` between event polls.
-- Returns a table with:
--   ok        boolean (exit_code == 0)
--   exit_code integer
--   stdout    string (concatenated stdout bytes)
--   stderr    string (concatenated stderr bytes)
--
-- The 10ms event-poll cadence is fast enough that git's output is
-- captured in chunks of a few-hundred bytes per tick on typical
-- repos. A more event-driven shape (an "events_wait" primitive that
-- blocks the coroutine until events arrive) would be cleaner; this
-- is what `V0.2-PREREQUISITES.md SP-2` (futures with `:await`) would
-- enable. v0.1 polls.

function M.run_git(args, opts)
  opts = opts or {}
  local id = pmacs.process.spawn {
    label = "git " .. (args[1] or ""),
    command = "git",
    args = args,
    cwd = opts.cwd,
  }
  local stdout_chunks, stderr_chunks = {}, {}
  local exit_code = nil
  while exit_code == nil do
    for _, ev in ipairs(pmacs.process.events_take(id)) do
      if ev.kind == "stdout" then
        stdout_chunks[#stdout_chunks + 1] = ev.bytes
      elseif ev.kind == "stderr" then
        stderr_chunks[#stderr_chunks + 1] = ev.bytes
      elseif ev.kind == "exited" then
        exit_code = ev.code
      end
      -- "started" / "ansi" events are ignored: ansi=false on the
      -- spec means we only get raw bytes; "started" carries no info
      -- run_git needs.
    end
    if exit_code == nil then
      pmacs.workers.sleep(10):await()
    end
  end
  -- After the exit event we may still have a tail of stdout buffered.
  -- Drain it once more so partial last-line output isn't lost.
  for _, ev in ipairs(pmacs.process.events_take(id)) do
    if ev.kind == "stdout" then
      stdout_chunks[#stdout_chunks + 1] = ev.bytes
    elseif ev.kind == "stderr" then
      stderr_chunks[#stderr_chunks + 1] = ev.bytes
    end
  end
  pmacs.process.forget(id)
  return {
    ok = (exit_code == 0),
    exit_code = exit_code,
    stdout = table.concat(stdout_chunks),
    stderr = table.concat(stderr_chunks),
  }
end

-- ---------------------------------------------------------------------------
-- Parsers (pure-Lua, no side effects)
-- ---------------------------------------------------------------------------

local function split_lines(text)
  local out = {}
  if text == nil or text == "" then return out end
  local i = 1
  while i <= #text + 1 do
    local nl = text:find("\n", i, true)
    if nl then
      out[#out + 1] = text:sub(i, nl - 1)
      i = nl + 1
    else
      if i <= #text then
        out[#out + 1] = text:sub(i)
      end
      break
    end
  end
  return out
end

-- Parse `git status --porcelain=v2 --branch` output.
-- Reference: https://git-scm.com/docs/git-status#_porcelain_format_version_2
--
-- Returns:
--   {
--     branch  = "main" or nil (from "# branch.head <name>" header),
--     staged    = array of "<status> <path>"  (X column non-space and non-?),
--     unstaged  = array of "<status> <path>"  (Y column non-space and non-?),
--     untracked = array of "<path>",          (lines starting with "?")
--   }
--
-- The status prefix is the porcelain-v2 XY pair as a single 2-char
-- string. For renames/copies, the path field shows "<new> -> <old>".
function M.parse_porcelain_v2(text)
  local out = {
    branch = nil,
    staged = {},
    unstaged = {},
    untracked = {},
  }
  for _, line in ipairs(split_lines(text)) do
    if line:sub(1, 1) == "#" then
      -- Header line. We only care about "# branch.head <name>".
      local head = line:match("^# branch%.head (.+)$")
      if head then out.branch = head end
    elseif line:sub(1, 1) == "?" then
      -- "? <path>" --- untracked.
      local path = line:match("^%? (.+)$")
      if path then out.untracked[#out.untracked + 1] = path end
    elseif line:sub(1, 1) == "1" or line:sub(1, 1) == "2" then
      -- "1 XY ..." (ordinary) or "2 XY ..." (rename/copy).
      -- Field 2 is the XY pair, field 9 (ordinary) or 10 (rename)
      -- is the path. We split by space and take the first two
      -- fields plus the trailing path (everything after the 8th /
      -- 9th space).
      local kind = line:sub(1, 1)
      local xy = line:sub(3, 4)
      local path
      if kind == "1" then
        -- "1 XY sub mH mI mW hH hI path" --- 5 fields between sub
        -- and path (mH, mI, mW, hH, hI).
        path = line:match("^1 .. .... %S+ %S+ %S+ %S+ %S+ (.+)$")
      else  -- kind == "2"
        -- "2 XY sub mH mI mW hH hI Xscore path<TAB>orig" --- 6
        -- fields between sub and path (the previous 5 + Xscore).
        path = line:match("^2 .. .... %S+ %S+ %S+ %S+ %S+ %S+ (.+)$")
      end
      if path then
        local x = xy:sub(1, 1)
        local y = xy:sub(2, 2)
        if x ~= "." and x ~= " " then
          out.staged[#out.staged + 1] = x .. " " .. path
        end
        if y ~= "." and y ~= " " then
          out.unstaged[#out.unstaged + 1] = y .. " " .. path
        end
      end
    elseif line:sub(1, 1) == "u" then
      -- "u XY ..." --- unmerged. Treat as both staged and unstaged.
      local xy = line:sub(3, 4)
      local path = line:match("^u .. .... .+ (%S+)$")
      if path then
        out.staged[#out.staged + 1] = xy .. " " .. path
      end
    end
  end
  return out
end

-- Parse `git log --oneline -n N` output.
-- Each line: "<short-hash> <subject>".
function M.parse_log(text)
  local out = {}
  for _, line in ipairs(split_lines(text)) do
    local hash, subject = line:match("^(%S+) (.*)$")
    if hash then
      out[#out + 1] = { hash = hash, subject = subject }
    end
  end
  return out
end

-- Parse `git branch --list` output.
-- Lines: "* <current-branch>" or "  <branch>".
function M.parse_branches(text)
  local out = { current = nil, all = {} }
  for _, line in ipairs(split_lines(text)) do
    local current = line:match("^%* (.+)$")
    if current then
      out.current = current
      out.all[#out.all + 1] = current
    else
      local name = line:match("^  (.+)$")
      if name then
        out.all[#out.all + 1] = name
      end
    end
  end
  return out
end

-- Parse `git stash list` output.
-- Lines: "stash@{N}: <branch>: <subject>".
function M.parse_stashes(text)
  local out = {}
  for _, line in ipairs(split_lines(text)) do
    local ref, subject = line:match("^(stash@{%d+}): (.*)$")
    if ref then
      out[#out + 1] = { ref = ref, subject = subject }
    end
  end
  return out
end

-- ---------------------------------------------------------------------------
-- Section-spec assembly
-- ---------------------------------------------------------------------------

local function placeholder_or_join(items, format_fn, empty_message)
  if #items == 0 then return empty_message end
  local lines = {}
  for i, item in ipairs(items) do
    lines[i] = format_fn(item)
  end
  return table.concat(lines, "\n")
end

-- Build a section spec from parsed git output. The spec follows the
-- M8.6 canonical 5-section layout: working tree, staged, recent
-- commits, branches, stashes. Section IDs are stable across
-- refreshes so fold-state survives.
--
-- `parsed` is a table with fields:
--   status   = parse_porcelain_v2 result
--   log      = parse_log result
--   branches = parse_branches result
--   stashes  = parse_stashes result
--
-- Empty sections render as a one-line placeholder body
-- ("(none)" / "(no stashes)" / etc.) per the M8.6 acceptance
-- bullet "Empty sections render as a one-line placeholder rather
-- than disappearing."
function M.build_spec(parsed)
  local status = parsed.status or { staged = {}, unstaged = {}, untracked = {} }
  local log = parsed.log or {}
  local branches = parsed.branches or { current = nil, all = {} }
  local stashes = parsed.stashes or {}

  -- "Working tree" combines unstaged-modified and untracked, mirroring
  -- magit's grouping. (Magit's actual UX has them as separate sections
  -- but the M8.6 spec lists "working-tree changes" as a single
  -- section; we follow the spec.)
  local working_items = {}
  for _, e in ipairs(status.unstaged) do
    working_items[#working_items + 1] = e
  end
  for _, p in ipairs(status.untracked) do
    working_items[#working_items + 1] = "?? " .. p
  end

  local staged_count = #status.staged
  local working_count = #working_items
  local commits_count = #log
  local branches_count = #branches.all
  local stashes_count = #stashes

  return {
    {
      id = "working",
      title = "Working tree changes (" .. working_count .. ")",
      body = placeholder_or_join(
        working_items,
        function(item) return item end,
        "(no working-tree changes)"
      ),
    },
    {
      id = "staged",
      title = "Staged changes (" .. staged_count .. ")",
      body = placeholder_or_join(
        status.staged,
        function(item) return item end,
        "(nothing staged)"
      ),
    },
    {
      id = "log",
      title = "Recent commits (" .. commits_count .. ")",
      body = placeholder_or_join(
        log,
        function(c) return c.hash .. " " .. c.subject end,
        "(no commits yet)"
      ),
    },
    {
      id = "branches",
      title = "Branches (" .. branches_count .. ")",
      body = placeholder_or_join(
        branches.all,
        function(name)
          if name == branches.current then return "* " .. name end
          return "  " .. name
        end,
        "(no branches)"
      ),
    },
    {
      id = "stashes",
      title = "Stashes (" .. stashes_count .. ")",
      body = placeholder_or_join(
        stashes,
        function(s) return s.ref .. ": " .. s.subject end,
        "(no stashes)"
      ),
    },
  }
end

-- ---------------------------------------------------------------------------
-- fetch_sections: orchestrate the 4 git invocations
-- ---------------------------------------------------------------------------
--
-- Sequential: status, log, branches, stashes. Each runs to
-- completion before the next starts. On a typical repo, this totals
-- well under 100 ms; on a pathologically large repo (tens of
-- thousands of dirty files), the working-tree section may dominate
-- but that's a `git status` problem, not a pmacs-magit problem.
--
-- A non-zero exit on any single invocation collapses the
-- corresponding section to its placeholder; we don't fail the whole
-- refresh. This means a transient git error (lockfile contention,
-- repo corruption mid-refresh) shows up as a reduced display, not a
-- crashed package. Stderr from failed invocations is captured but
-- not surfaced to the visible buffer in v0.1; future revisions
-- might add an "errors" section.
--
-- Must be called inside `pmacs.async`.

function M.fetch_sections(repo_root)
  local opts = { cwd = repo_root }
  local function safe_parse(parser, result)
    if not result.ok then return nil end
    return parser(result.stdout)
  end

  local status_res = M.run_git({ "status", "--porcelain=v2", "--branch" }, opts)
  local log_res = M.run_git({ "log", "--oneline", "-n", "10" }, opts)
  local branches_res = M.run_git({ "branch", "--list" }, opts)
  local stashes_res = M.run_git({ "stash", "list" }, opts)

  local parsed = {
    status = safe_parse(M.parse_porcelain_v2, status_res),
    log = safe_parse(M.parse_log, log_res),
    branches = safe_parse(M.parse_branches, branches_res),
    stashes = safe_parse(M.parse_stashes, stashes_res),
  }
  return {
    spec = M.build_spec(parsed),
    parsed = parsed,
  }
end

return M
