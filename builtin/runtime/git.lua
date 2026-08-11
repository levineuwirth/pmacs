-- git.lua --- Git integration Stage 1: read-only status and diff.
-- Framing: docs/git-integration-framing.md (revision 5).
--
-- Two surfaces and nothing else:
--
--   *git-status*  a `pmacs.listview` panel over
--                 `git --no-optional-locks -C <root> status
--                  --porcelain=v2 --branch -z`. RET visits the file,
--                 `d` shows its diff, `g` refreshes.
--   *git-diff*    the diff for the FILE under point, in a generated
--                 buffer rendered as plain text. There is no bundled
--                 `diff` grammar (checked: `BUILTIN_LANGUAGES` in
--                 src/syntax.rs has no entry), and there is no hunk
--                 model anywhere in the tree --- hunks are what gutter
--                 markers need, and that is Stage 2's protocol work.
--
-- NO WIRE CHANGE. Stage 2 (gutter markers) needs new `DecorationKind`
-- variants and a `PROTOCOL_VERSION` bump, and must be scheduled alone.
--
-- Three facts this module is built on, each measured rather than
-- reasoned about:
--
--  * `ProjectKind::Git` means a BARE repository ("no language marker
--    found inside", src/project.rs), and a language marker beside
--    `.git` WINS --- so pmacs reports `kind = "rust"` for its own
--    repository. This module therefore never asks pmacs whether
--    something is a git repo: it runs `git -C <dir> rev-parse
--    --show-toplevel` and lets a non-zero exit be the answer. Git's own
--    resolution handles submodules, worktrees, `GIT_DIR` and `.git`
--    files; a marker walk reimplements a subset and gets it wrong.
--  * `git diff --no-index` implies `--exit-code`: it exits 1 when it
--    SUCCESSFULLY finds differences. For the untracked path the success
--    predicate is therefore exit in {0, 1}; only >= 2 is a failure.
--    That asymmetry is confined to `--no-index`.
--  * An unborn `HEAD` makes `git diff HEAD` exit 128 with
--    `fatal: bad revision 'HEAD'`. It is detected from
--    `# branch.oid (initial)` in the `--branch` output this module
--    already parses --- NOT from a second `rev-parse` process for a
--    fact the first one hands over.
--
-- Background-work attribution is a NEGATIVE (COHERENCE.md §9): a
-- spawned process does not appear in `*workers*` at all --- that buffer
-- is `async.lua`'s job list, while processes live under
-- `pmacs.process.list`. Every spawn here is labelled, which is strictly
-- better than an anonymous `git`, but a label is not attribution and
-- this module does not pretend otherwise. Accepted because these are
-- short-lived reads.

pmacs.git = pmacs.git or {}

local STATUS_PANEL = "*git-status*"
local DIFF_BUFFER = "*git-diff*"

--- The git program name.
---
--- A module-local rather than a setting: Q#G-4 defines exactly one
--- setting and says to resist more until there is use evidence. It is
--- reachable from Lua because the missing-binary path (Q#G-2's named
--- risk) has to be WITNESSED, and there is no other way to reach it in
--- process: Rust's `Command` resolves the program against the PARENT
--- process's `PATH`, so a child `env` cannot hide git, and
--- `std::env::set_var` is `unsafe` in edition 2024, which this project
--- forbids. Pointing this at a name that is not on `PATH` produces
--- exactly the ENOENT a missing git produces.
pmacs.git._program = "git"

--- The most recent spawn's `{ command, args, cwd, label }`.
---
--- Public because the `--no-optional-locks` contract is witnessed
--- STRUCTURALLY: a lock that was not taken cannot be observed directly,
--- so the assembled invocation is what gets pinned (Q#G-6).
pmacs.git._last_spawn = nil

--- The last few spawns' argv, oldest first.
---
--- A bounded diagnostic ring rather than an unbounded log. It exists
--- because one of this module's contracts is about a process that must
--- NOT be spawned: unborn `HEAD` is detected from `# branch.oid
--- (initial)` in output already in hand, and nobody should later
--- reintroduce a `rev-parse --verify HEAD` for it. "Which processes did
--- that open run?" is not answerable from `_last_spawn` alone.
pmacs.git._spawn_log = {}

local SPAWN_LOG_LIMIT = 16

-- ---------------------------------------------------------------------
-- Configuration (Q#G-4)
-- ---------------------------------------------------------------------

pmacs.config.define {
  name = "git.enabled",
  description = "Whether the Git commands (*git-status*, *git-diff*) run git at all. Turning this off makes them report that they are disabled rather than spawning anything.",
  type = "boolean",
  default = true,
  mutability = "live",
}

local function git_enabled()
  local ok, value = pcall(pmacs.config.get, "git.enabled")
  if not ok then return true end
  return value ~= false
end

-- ---------------------------------------------------------------------
-- Text safety at the binding boundary (Q#G-8)
-- ---------------------------------------------------------------------
--
-- Git hands back PATH BYTES. `pmacs.process.spawn` takes
-- `args: Vec<String>` and `pmacs.buffer.find_or_open` takes
-- `path: String` --- both Rust `String`, i.e. UTF-8 by construction ---
-- and the rope is UTF-8 by project invariant, so a path that is valid
-- bytes but not valid UTF-8 can be READ and DISPLAYED and cannot be
-- passed back for a diff, nor opened. The honest boundary is: parse it,
-- show it (escaped, since the raw bytes cannot enter a rope), and
-- REFUSE the gesture with a message.

-- Length of the valid UTF-8 sequence starting at byte `i`, or nil.
-- Rejects overlongs, surrogates and anything past U+10FFFF, so
-- "displayable" means the same thing here as it does to Rust.
local function utf8_seq_len(s, i)
  local b1 = s:byte(i)
  if not b1 then return nil end
  if b1 < 0x80 then return 1 end
  local n, cp
  if b1 >= 0xC2 and b1 <= 0xDF then
    n, cp = 2, b1 - 0xC0
  elseif b1 >= 0xE0 and b1 <= 0xEF then
    n, cp = 3, b1 - 0xE0
  elseif b1 >= 0xF0 and b1 <= 0xF4 then
    n, cp = 4, b1 - 0xF0
  else
    return nil
  end
  if i + n - 1 > #s then return nil end
  for k = 1, n - 1 do
    local b = s:byte(i + k)
    if b < 0x80 or b > 0xBF then return nil end
    cp = cp * 64 + (b - 0x80)
  end
  if n == 3 and cp < 0x800 then return nil end
  if n == 4 and cp < 0x10000 then return nil end
  if cp >= 0xD800 and cp <= 0xDFFF then return nil end
  if cp > 0x10FFFF then return nil end
  return n
end

--- True when `s` is valid UTF-8, i.e. when it can cross the binding
--- boundary at all.
function pmacs.git.is_text(s)
  if type(s) ~= "string" then return false end
  local i, n = 1, #s
  while i <= n do
    local len = utf8_seq_len(s, i)
    if not len then return false end
    i = i + len
  end
  return true
end

--- `s` rendered for a ONE-LINE panel row: invalid UTF-8 bytes and every
--- control byte become `\xNN`.
---
--- Escaping controls is not cosmetic. A path may contain a newline ---
--- that is exactly what `-z` buys and what a quoted parser gets wrong
--- --- and a raw newline in a row would split it across two lines and
--- desynchronize every line-to-row mapping in the panel.
function pmacs.git.display_path(s)
  if type(s) ~= "string" then return "" end
  local out = {}
  local i, n = 1, #s
  while i <= n do
    local len = utf8_seq_len(s, i)
    local b = s:byte(i)
    if len and not (len == 1 and (b < 0x20 or b == 0x7F)) then
      out[#out + 1] = s:sub(i, i + len - 1)
      i = i + len
    else
      out[#out + 1] = string.format("\\x%02X", b)
      i = i + 1
    end
  end
  return table.concat(out)
end

--- `s` with invalid UTF-8 bytes replaced by U+FFFD, controls left
--- alone. For MULTI-LINE bodies (a patch), where newlines and tabs are
--- content rather than a hazard.
local function utf8_clean(s)
  if pmacs.git.is_text(s) then return s end
  local out = {}
  local i, n = 1, #s
  while i <= n do
    local len = utf8_seq_len(s, i)
    if len then
      out[#out + 1] = s:sub(i, i + len - 1)
      i = i + len
    else
      out[#out + 1] = "\239\191\189" -- U+FFFD
      i = i + 1
    end
  end
  return table.concat(out)
end

-- ---------------------------------------------------------------------
-- Running git (Q#G-2)
-- ---------------------------------------------------------------------

--- The argv for a git invocation rooted at `root`.
---
--- Every call site goes through here, so `--no-optional-locks` cannot
--- be dropped by one of them. The flag is part of the contract, not a
--- nicety (Q#G-6): `git status` is not strictly read-only --- it may
--- refresh and write the index --- and this module runs it
--- asynchronously from an editor while the user may be running git in a
--- terminal, which is the exact scenario the flag exists for.
function pmacs.git.argv(root, rest)
  local args = { "--no-optional-locks" }
  if root then
    args[#args + 1] = "-C"
    args[#args + 1] = root
  end
  for _, a in ipairs(rest) do args[#args + 1] = a end
  return args
end

-- proc raw id -> { procid, out, err, on_done }
local pump = {}

-- Spawn git and call `on_done { ok, code, kind, stdout, stderr }` once
-- it terminates. A spawn failure calls `on_done` too, with
-- `spawn_error` set --- §1.2's silence asymmetry: the failure must be
-- surfaced with guidance, never swallowed.
local function run_git(label, purpose, root, rest, on_done)
  local spec = {
    label = label,
    -- Required since worker identity Stage 1 (#232), and deliberately
    -- NOT the label. `label` identifies the process --- "git status" ---
    -- while this says what the run is FOR, which is what someone reading
    -- `*workers*` or the statusline activity indicator needs: three of
    -- this module's spawns are all "git", and only the purpose tells
    -- them apart. Each call site writes its own; copying the label
    -- across is the failure that ruling was made against.
    purpose = purpose,
    command = pmacs.git._program,
    args = pmacs.git.argv(root, rest),
    stdin = "null",
  }
  if root then spec.cwd = root end
  pmacs.git._last_spawn = {
    command = spec.command, args = spec.args, cwd = spec.cwd, label = spec.label,
    purpose = spec.purpose,
  }
  local log = pmacs.git._spawn_log
  log[#log + 1] = spec.args
  while #log > SPAWN_LOG_LIMIT do table.remove(log, 1) end
  local ok, proc = pcall(pmacs.process.spawn, spec)
  if not ok then
    on_done {
      ok = false, kind = "spawn_failed", spawn_error = tostring(proc),
      stdout = "", stderr = "",
    }
    return nil
  end
  pump[proc:raw()] = { procid = proc, out = {}, err = {}, on_done = on_done }
  return proc
end

pmacs.hook.add("process.after-tick", function()
  for raw, entry in pairs(pump) do
    for _, ev in ipairs(pmacs.process.events_take(entry.procid)) do
      local kind = ev.kind
      if kind == "stdout" then
        entry.out[#entry.out + 1] = ev.bytes
      elseif kind == "stderr" then
        entry.err[#entry.err + 1] = ev.bytes
      elseif kind == "exited" or kind == "signaled" or kind == "crashed" then
        -- The supervisor drains all remaining output BEFORE pushing the
        -- terminal event (`final_drain_runtime`, src/process.rs), so one
        -- pass in event order captures everything.
        pump[raw] = nil
        pcall(pmacs.process.forget, entry.procid)
        local result = {
          ok = (kind == "exited"),
          code = (kind == "exited") and (ev.code or 0) or nil,
          kind = kind,
          signal = ev.signal,
          error = ev.error,
          stdout = table.concat(entry.out),
          stderr = table.concat(entry.err),
        }
        local called, err = pcall(entry.on_done, result)
        if not called then
          pmacs.editor.set_status("git: " .. tostring(err))
        end
      end
    end
  end
end)

--- The first line of `text`, trimmed, or `""`.
---
--- For text bound for the ONE-LINE STATUS BAND, and only for that: a
--- spawn error, a stderr detail, an error string. A status message that
--- carried a newline would corrupt the row layout of whatever is
--- rendering it, so truncating to the first line is the right answer
--- there.
local function first_line(text)
  local line = (text or ""):match("^[^\r\n]*") or ""
  return (line:gsub("%s+$", ""))
end

--- `text` with git's final output terminator removed, and NOTHING else.
---
--- The counterpart to `first_line`, deliberately a SECOND function
--- rather than a change to it, because the two answer opposite
--- questions and each has callers that the other's answer would break.
--- This one is for COMMAND OUTPUT that must survive whole: a POSIX path
--- may legally contain a newline, so `git rev-parse --show-toplevel`
--- prints one for a repository rooted at `/tmp/a\nb`, and taking the
--- first line there truncates the root to `/tmp/a` --- after which every
--- command this module runs has a wrong or nonexistent cwd. `first_line`
--- has three other callers, all of them status-band text, and folding
--- the two together would fix this one and break those.
---
--- Exactly ONE trailing `\n` is stripped, and NOTHING may ride along
--- with it --- not a carriage return, not a second newline. A carriage
--- return is as legal a POSIX path byte as a newline is, so a repository
--- rooted at `/tmp/a\r` makes git print `/tmp/a` `0d` `0a`: the path's
--- own CR, then git's LF terminator. A strip tolerant of `\r?\n$` cannot
--- tell those two bytes apart and takes both, resolving the root as
--- `/tmp/a` --- the same defect this function was written to fix, one
--- byte over. A second newline is output, not a terminator, for the same
--- reason.
---
--- There is no unambiguous output representation to prefer instead,
--- which was CHECKED against git 2.55 rather than assumed: `-z` is not
--- an option of `git rev-parse` at all. It is absent from the manual,
--- `--parseopt -z` errors with "unknown switch", and in ordinary mode
--- `rev-parse` treats `-z` as an unrecognized FLAG ARGUMENT and echoes a
--- literal `-z\n` onto stdout AHEAD of the toplevel --- so asking for it
--- would corrupt the very output it was meant to disambiguate, silently
--- and with exit code 0. `--show-toplevel` applies no C quoting either,
--- not even under `core.quotePath=true`. Removing the one byte git
--- appended is therefore the whole of the correct answer.
---
--- Written as an explicit last-byte test rather than a pattern: an
--- anchored Lua pattern is where both of this function's bugs lived.
local function strip_output_terminator(text)
  text = text or ""
  if text:sub(-1) == "\n" then return text:sub(1, -2) end
  return text
end

--- A one-line description of why a git invocation failed.
local function failure_reason(res)
  if res.spawn_error then
    return string.format(
      "cannot run %q (%s) --- is git installed and on PATH?",
      pmacs.git._program, first_line(res.spawn_error))
  end
  local detail = first_line(utf8_clean(res.stderr))
  if res.kind == "signaled" then
    return string.format("git was killed by %s%s",
      res.signal or "a signal", detail ~= "" and (": " .. detail) or "")
  end
  if res.kind == "crashed" then
    return string.format("git crashed: %s", res.error or detail)
  end
  return string.format("git exited with code %d%s",
    res.code or -1, detail ~= "" and (": " .. detail) or "")
end

-- ---------------------------------------------------------------------
-- Porcelain v2 parsing (Q#G-6)
-- ---------------------------------------------------------------------
--
-- The SEPARATION here --- pure `parse_*` functions that take a string
-- and return structure, testable with no repository --- is ported from
-- `tests/fixtures/pmacs-magit/status.lua`, whose 32-test suite proves
-- the shape works. The record TOKENIZER is deliberately NOT ported: the
-- fixture reads newline-delimited v2 and this reads `-z`, and those are
-- different grammars. Under `-z` a record's fields are NUL-terminated,
-- so a rename carries its two paths as SEPARATE fields rather than
-- tab-joined inside one, and C quoting is removed from the problem
-- entirely rather than obliging a hand-written unquoter.
--
-- The fixture is left untouched (Q#G-0): its purpose is to prove the
-- PACKAGE SYSTEM can host this, and bundled code becoming its
-- dependency would make `tests/m8_6_acceptance.rs` test less than it
-- claims. The duplication is deliberate and stated rather than quiet.

-- Split NUL-terminated fields. A trailing empty fragment after the last
-- NUL is dropped; an empty payload yields no fields.
local function nul_fields(text)
  local out = {}
  if type(text) ~= "string" or text == "" then return out end
  local i = 1
  while true do
    local nul = text:find("\0", i, true)
    if not nul then
      if i <= #text then out[#out + 1] = text:sub(i) end
      break
    end
    out[#out + 1] = text:sub(i, nul - 1)
    i = nul + 1
  end
  return out
end

--- Parse `git status --porcelain=v2 --branch -z` output.
---
--- Returns `{ branch = {...}, rows = {...} }` where each row is
---
---   { kind = "ordinary"|"rename"|"unmerged"|"untracked"|"ignored",
---     xy, x, y, path, orig, score }
---
--- `path` is the CURRENT path --- what the panel shows and what RET
--- visits --- and `orig` remembers where a rename or copy came from.
--- Both are raw bytes; nothing here assumes they are text.
---
--- **`kind = "rename"` covers copies too, deliberately.** A `2` record
--- is porcelain-v2's ONE two-path record and every behaviour keyed on
--- it is the same for both classes --- notably the two-path
--- `git diff HEAD -- <orig> <current>`. What differs is only what the
--- user is TOLD, and that fact is already carried: `score` leads with
--- `R` or `C` (and `xy` carries the same letter on whichever side
--- detected it). See `is_copy` for where the two are told apart.
function pmacs.git.parse_status(text)
  local branch = { unborn = false }
  local rows = {}
  local fields = nul_fields(text)
  local i = 1
  while i <= #fields do
    local field = fields[i]
    local tag = field:sub(1, 1)
    if tag == "#" then
      local key, value = field:match("^# (%S+) (.*)$")
      if key == "branch.oid" then
        branch.oid = value
        -- Unborn HEAD, from the output already being parsed. A second
        -- `rev-parse --verify HEAD` would be a whole extra process for
        -- a fact this line hands over.
        branch.unborn = (value == "(initial)")
      elseif key == "branch.head" then
        branch.head = value
      elseif key == "branch.upstream" then
        branch.upstream = value
      elseif key == "branch.ab" then
        branch.ahead = tonumber(value:match("^%+(%d+)") or "")
        branch.behind = tonumber(value:match("%-(%d+)$") or "")
      end
    elseif tag == "1" then
      -- 1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>
      local xy, path = field:match("^1 (%S%S) %S+ %S+ %S+ %S+ %S+ %S+ (.+)$")
      if xy then
        rows[#rows + 1] = {
          kind = "ordinary", xy = xy, x = xy:sub(1, 1), y = xy:sub(2, 2), path = path,
        }
      end
    elseif tag == "2" then
      -- 2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <Xscore> <path>\0<origPath>
      --
      -- The origin is the NEXT field, not a tab-joined suffix. That is
      -- the whole reason this tokenizer is not the fixture's.
      local xy, score, path =
        field:match("^2 (%S%S) %S+ %S+ %S+ %S+ %S+ %S+ (%S+) (.+)$")
      if xy then
        i = i + 1
        rows[#rows + 1] = {
          kind = "rename", xy = xy, x = xy:sub(1, 1), y = xy:sub(2, 2),
          path = path, orig = fields[i], score = score,
        }
      end
    elseif tag == "u" then
      -- u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>
      local xy, path =
        field:match("^u (%S%S) %S+ %S+ %S+ %S+ %S+ %S+ %S+ %S+ (.+)$")
      if xy then
        rows[#rows + 1] = {
          kind = "unmerged", xy = xy, x = xy:sub(1, 1), y = xy:sub(2, 2), path = path,
        }
      end
    elseif tag == "?" then
      local path = field:match("^%? (.+)$")
      if path then
        rows[#rows + 1] = { kind = "untracked", xy = "??", x = "?", y = "?", path = path }
      end
    elseif tag == "!" then
      local path = field:match("^! (.+)$")
      if path then
        rows[#rows + 1] = { kind = "ignored", xy = "!!", x = "!", y = "!", path = path }
      end
    end
    i = i + 1
  end
  return { branch = branch, rows = rows }
end

-- ---------------------------------------------------------------------
-- Request channels: "the newest INVOCATION wins"
-- ---------------------------------------------------------------------
--
-- EVERY defect review found on this module was one shape: module-level
-- mutable state read or written at CONTINUATION time without an
-- invocation-time ticket. So the rule gets one implementation instead of
-- a bespoke counter per call site.
--
-- A channel hands out a ticket when the user ASKS for something and
-- answers "is this still the request in force?" when a subprocess
-- finally replies. A continuation holding a stale ticket must discard
-- BEFORE ANY EFFECT --- no spawn, no shared-state write, and no status
-- message either, since a message from a replaced invocation is as wrong
-- as a panel from one.
--
-- There are TWO channels, and that is deliberate rather than an
-- oversight: a single module-wide counter would make pressing `d` cancel
-- an in-flight `g`, and vice versa. The status panel and the diff view
-- are independent things a user can ask for, so each gets its own "newest
-- wins" ordering. What is shared is the MECHANISM, not the counter.
--
-- A channel spans a whole request, not one process: a status open is
-- `rev-parse` then `status`, and a diff is one or two `git diff` runs.
-- Every stage of one request carries the ticket reserved at the command.
local function new_channel()
  local ch = { current = 0 }
  --- Claim the newest ticket, and return it. Called at the point a user
  --- ASKS for something, never at the point some subprocess answers ---
  --- minting on arrival makes the SLOWEST subprocess win instead of the
  --- newest invocation, which is exactly the bug this exists to stop.
  function ch.reserve()
    ch.current = ch.current + 1
    return ch.current
  end
  --- True while `ticket` is still the request in force.
  function ch.is_current(ticket)
    return ticket == ch.current
  end
  return ch
end

local status_requests = new_channel()
local diff_requests = new_channel()

-- ---------------------------------------------------------------------
-- Panel state
-- ---------------------------------------------------------------------

-- `display` maps a panel DATA LINE (1-based; the header is line 0) to
-- the git row rendered there. It is this module's own copy because
-- listview's line map is private, and `d` needs the row under the
-- cursor. `rows` is the same information as an array, for re-seating.
local state = {
  root = nil,
  branch = nil,
  rows = {},
  display = {},
  buffer = nil,
  diff_buffer = nil,
  failure = nil,
}

-- A copy and a rename are already TOLD APART here, and by the field
-- that tells every other row class apart: the `XY` prefix reads `R.`
-- for one and `C.` for the other, out of the same byte `score` leads
-- with. So this renders both the same way on purpose --- `<-` reads
-- "came from", which is true of a copy --- rather than growing a second
-- vocabulary beside the porcelain codes the whole panel is built on.
local function status_line_text(row)
  local shown = pmacs.git.display_path(row.path)
  if row.orig then
    shown = string.format("%s  <- %s", shown, pmacs.git.display_path(row.orig))
  end
  return string.format("%s  %s", row.xy, shown)
end

local function status_header()
  -- A failed run has no branch and no rows, and rendering "0 changes"
  -- above a failure row would be a small lie in the one place the panel
  -- most needs to be honest.
  if state.failure then
    return "git: status failed   g retry  q quit"
  end
  local branch = state.branch or {}
  local where
  if branch.unborn then
    where = string.format("%s (no commits yet)", branch.head or "HEAD")
  elseif branch.head == "(detached)" or branch.head == nil then
    where = string.format("detached at %s", (branch.oid or "?"):sub(1, 8))
  else
    where = branch.head
  end
  local n = #state.rows
  return string.format(
    "git: %s --- %d change%s   RET visit  d diff  n/p move  g refresh  q quit",
    where, n, n == 1 and "" or "s")
end

-- Build the listview rows and (re)build `state.display` alongside them,
-- so the two can never drift.
local function listview_rows(extra_text)
  state.failure = nil
  state.display = {}
  local out = {}
  for _, row in ipairs(state.rows) do
    out[#out + 1] = { text = status_line_text(row), item = row }
    state.display[#out] = row
  end
  if extra_text then
    out[#out + 1] = { text = extra_text }
  end
  return out
end

-- Row-level failure (Q#G-1 item 4): a failure is a ROW, not a silence.
local function failure_rows(reason)
  state.failure = reason
  state.rows = {}
  state.display = {}
  return { { text = "! " .. reason } }
end

-- ---------------------------------------------------------------------
-- Visiting (RET)
-- ---------------------------------------------------------------------

local function join_root(path)
  if path:sub(1, 1) == "/" then return path end
  return (state.root or ".") .. "/" .. path
end

local function refuse_unrepresentable(row)
  pmacs.editor.set_status(string.format(
    "git: %s is not valid UTF-8, so pmacs cannot open or diff it",
    pmacs.git.display_path(row.path)))
end

local function visit_row(row)
  if type(row) ~= "table" or not row.path then return end
  if not pmacs.git.is_text(row.path) then
    refuse_unrepresentable(row)
    return
  end
  local target = join_root(row.path)
  pmacs.editor.push_jump()
  -- A visit FROM a panel lands in the DOCUMENT target and leaves the
  -- panel where it is (Q#BP11b) --- `display_file`, never the raw
  -- switch, which would clobber the panel with the source.
  local ok, err = pcall(pmacs.window.display_file, target, { select = true })
  if not ok then
    pmacs.editor.jump_back()
    pmacs.editor.set_status(string.format("git: cannot open %s: %s",
      pmacs.git.display_path(row.path), first_line(tostring(err))))
  end
end

-- ---------------------------------------------------------------------
-- The status refresh (Q#G-1)
-- ---------------------------------------------------------------------

local function open_status_panel(rows)
  pmacs.listview.open {
    name = STATUS_PANEL,
    header = status_header(),
    rows = rows,
    -- `d` is not on listview's key surface (RET SPC n <down> p <up> TAB
    -- g q), and it cannot be bound from outside the primitive safely:
    -- a name collision disambiguates to `<2>`, so the name passed here
    -- is not necessarily the buffer that came back. The `keys` table
    -- binds it through the primitive's own buffer-local path, so no key
    -- is intercepted and COHERENCE.md §6 stays at six shadows.
    keys = { d = "git.diff-file" },
    on_visit = visit_row,
    on_refresh = function() return pmacs.git._on_refresh() end,
  }
  -- `listview.open` takes `select = true`, so the panel is the active
  -- buffer here. Captured rather than looked up by name, because the
  -- name may have been disambiguated.
  state.buffer = pmacs.window.buffer()
end

-- ---------------------------------------------------------------------
-- The destination boundary (Q#JR14, Q#DC-1)
-- ---------------------------------------------------------------------
--
-- Every continuation in this module renders a tick or more after the
-- keypress that asked for it, and until #231 there was nothing it could
-- render *to* except ambient state: whichever frontend happened to be
-- active when git exited. Run `M-x git.status` in frontend A, let B
-- become active while `git status` runs, and A's panel opened in B.
--
-- So the destination is CAPTURED AT INVOCATION and threaded through,
-- exactly as this module already threads the generation, the root and
-- the row --- and for the same reason. `state.root` has a comment
-- explaining why reading module-level state per step is wrong; the
-- ambient frontend is that same argument one layer down, and it was the
-- one thing still being read late.

--- Run `body` against the destination captured at invocation.
---
--- Returns false when the commit is REFUSED --- the captured window or
--- buffer is gone, or the frontend can no longer satisfy `profile`.
--- A refusal drops the render, which is the same answer the
--- `expect_buffer` rule already gives when the panel a refresh belongs
--- to has been killed: a result whose destination no longer exists is
--- not a result to force somewhere else. `commit_to` refuses BEFORE the
--- body runs, so a refusal is mutation-free and there is no partial
--- render to undo.
---
--- Deliberately not `pcall`-wrapped. A refusal returns `(false, reason)`
--- and is handled here; anything that RAISES is a defect in this module
--- (a fabricated destination, an unknown profile) and must not be
--- swallowed into a silent no-op.
local function commit_ui(dest, profile, body)
  local ok, reason = pmacs.window.commit_to(dest, body, profile)
  if ok == false then return false, reason end
  return true
end

--- The status-refresh ticket currently in force.
---
--- Exposed alongside `_deliver_status` below, and for the same reason:
--- the discard rule is about a completion arriving LATE, and a caller
--- cannot construct a stale request without knowing what "current"
--- means.
function pmacs.git._generation()
  return status_requests.current
end

--- The diff ticket currently in force. `_generation`'s counterpart, on
--- the other channel; see `new_channel` for why they are two.
function pmacs.git._diff_generation()
  return diff_requests.current
end

--- Deliver a completed `git status`.
---
--- Exposed because concurrent refresh is asserted by DRIVING two
--- refreshes and completing them out of order, which no arrangement of
--- real subprocess timing can guarantee.
function pmacs.git._deliver_status(request, res)
  -- Generation (Q#G-1 item 3): a second `g` while one is in flight
  -- bumps the ticket, and the older completion DISCARDS its rows rather
  -- than racing. It does not terminate the first process --- reaping is
  -- `process.forget`'s job and killing git mid-read buys nothing.
  if not status_requests.is_current(request.generation) then return end
  -- Panel lifetime (Q#G-1 item 5): if the buffer this refresh belongs
  -- to is gone, drop the result. A FIRST open carries no expectation.
  if request.expect_buffer ~= nil then
    local live, valid = pcall(request.expect_buffer.is_valid, request.expect_buffer)
    if not (live and valid) then return end
  end

  -- Rows and the status line are COMPUTED here and EMITTED inside the
  -- commit below. Splitting them is the point: `set_status` is a UI
  -- mutation, and a failure message announcing a panel that the commit
  -- then refuses to open is the misrouting this boundary exists to
  -- prevent, in its most confusing form.
  local rows, message
  if res.ok and res.code == 0 then
    local parsed = pmacs.git.parse_status(res.stdout)
    state.branch = parsed.branch
    state.rows = parsed.rows
    rows = listview_rows(nil)
  else
    local reason = failure_reason(res)
    state.branch = state.branch or {}
    rows = failure_rows(reason)
    message = "git status: " .. reason
  end

  -- The PANEL profile: `*git-status*` is a bottom-panel surface
  -- (`listview.open` defaults `display` to `"panel"`), so the
  -- stale-intent checks that guard replacing a document window do not
  -- apply --- but only while the placement really is a panel, which is
  -- what the profile's own refusal keeps true for the extent of this
  -- body (Q#DC-2).
  commit_ui(request.dest, "panel", function()
    if message then pmacs.editor.set_status(message) end
    open_status_panel(rows)
    -- `listview.open` resets collapse and does NOT preserve selection ---
    -- only `listview.refresh` does, and that is the synchronous path this
    -- model cannot use. So re-seating is owned here.
    --
    -- And `open`'s own `seat_cursor(p, 1)` cannot be relied on either: it
    -- walks DOWN from wherever the cursor is, on the premise that a fresh
    -- `switch_active_buffer` zeroed it. Re-opening a panel that is
    -- already displayed does not zero anything, so that walk would land
    -- one row below the previous cursor instead of on row 1. The handler
    -- therefore seats unconditionally, from line 0.
    local target = 1
    if request.selected_path then
      for i, row in ipairs(state.rows) do
        if row.path == request.selected_path then
          target = i
          break
        end
      end
    end
    -- If the captured path is gone --- the commonest case, since a file
    -- that stopped being modified drops out of status --- `target` stays
    -- 1 and nothing is said about it. That is the correct answer, not a
    -- failure.
    pmacs.editor.clear_selection()
    pmacs.editor.set_view_top(0)
    pmacs.editor.move_to_line(0)
    -- Walked with `move_down` rather than a single `move_to_line(target)`
    -- because motion is what drags the viewport along; a bare cursor set
    -- would leave a long status list scrolled to the top with the cursor
    -- off screen. This is `listview.refresh`'s own idiom.
    for _ = 1, target do pmacs.editor.move_down() end
  end)
end

-- The path of the row under the cursor right now, or nil.
local function selected_path()
  local row = state.display[pmacs.editor.cursor_line()]
  return row and row.path or nil
end

--- Spawn `git status` under an ALREADY-RESERVED generation.
---
--- The generation is a parameter rather than something minted here,
--- because this runs from the root-resolution callback: two `git.status`
--- invocations against different repositories resolve their roots
--- concurrently, and if the generation were minted on arrival then the
--- invocation whose `rev-parse` returned LAST would claim the newest
--- generation and replace the newer request. Reserving at the command
--- and carrying it through is what makes the ordering the user's, not
--- the filesystem's.
local function start_status(root, expect_buffer, want_selection, generation, dest)
  local request = {
    generation = generation,
    expect_buffer = expect_buffer,
    selected_path = want_selection and selected_path() or nil,
    -- A parameter for the same reason `generation` is: it belongs to
    -- the invocation, and this function can run from a root-resolution
    -- callback that is already a tick removed from it.
    dest = dest,
  }
  run_git("git status",
    "reading the working tree status of " .. root .. " for the *git-status* panel",
    root,
    { "status", "--porcelain=v2", "--branch", "-z" },
    function(res) pmacs.git._deliver_status(request, res) end)
end

--- `g` inside the panel.
---
--- `on_refresh` stays SYNCHRONOUS and honest: it returns the current
--- rows immediately with a marker appended and KICKS OFF the spawn, so
--- `g` always re-renders and always shows that work started. A `g` that
--- silently does nothing is a defect this primitive already names.
function pmacs.git._on_refresh()
  if not git_enabled() then
    return listview_rows("(git.enabled is false --- nothing was run)")
  end
  if not state.root then
    return listview_rows("(no repository --- run M-x git.status)")
  end
  -- Reserved HERE, at the keypress, for the same reason `git.status`
  -- reserves at the command: `g` needs no root lookup, so this is
  -- already the moment of invocation. The destination is captured on
  -- the same line of reasoning --- `g` is pressed IN the panel, so the
  -- frontend showing it is the one this refresh belongs to, and that is
  -- true now and possibly not true when git exits.
  start_status(state.root, state.buffer, true, status_requests.reserve(),
    pmacs.window.capture_destination())
  return listview_rows("(refreshing...)")
end

-- ---------------------------------------------------------------------
-- Entry point
-- ---------------------------------------------------------------------

local function directory_of(path)
  local dir = path:match("^(.*)/[^/]*$")
  if dir == nil or dir == "" then return "/" end
  return dir
end

--- The directory a status run should resolve its repository from.
---
--- The ACTIVE FILE's directory, falling back to the daemon's working
--- directory when the active buffer is pathless (a dired listing, the
--- scratch buffer). Deliberately not a project-marker walk: git resolves
--- its own worktree, and `ProjectKind` cannot answer the question at all.
local function active_directory()
  local buf = pmacs.window.buffer()
  if buf then
    local ok, path = pcall(function() return buf:path() end)
    if ok and type(path) == "string" and path ~= "" then
      return directory_of(path)
    end
  end
  local ok, id = pcall(pmacs.instance.identity)
  if ok and type(id) == "table" and type(id.working_directory) == "string" then
    return id.working_directory
  end
  return nil
end

--- Deliver a completed root lookup for the invocation in `request`.
---
--- Exposed for the same reason `_deliver_status` is: the contract is
--- about completions arriving in an order the CALLER did not choose, and
--- no arrangement of real subprocess timing can guarantee that two
--- `rev-parse` runs finish in a chosen order.
function pmacs.git._deliver_root(request, res)
  -- A root lookup that returns after a newer invocation has superseded
  -- it must not proceed: not to a status spawn, not to `state.root`, and
  -- not even to a status-line message. Everything below this line is an
  -- effect belonging to an invocation the user has already replaced.
  if not status_requests.is_current(request.generation) then return end

  if not (res.ok and res.code == 0) then
    if res.spawn_error then
      pmacs.editor.set_status("git: " .. failure_reason(res))
    else
      pmacs.editor.set_status(
        string.format("git: %s is not inside a repository", request.dir))
    end
    return
  end
  -- The WHOLE output, minus its terminator --- never the first line, and
  -- never a byte more than git appended. A repository root may contain a
  -- newline and may END in a carriage return, and losing either here
  -- would point every command that follows at a directory that does not
  -- exist.
  local root = strip_output_terminator(res.stdout)
  if root == "" then
    pmacs.editor.set_status("git: rev-parse returned no worktree root")
    return
  end
  state.root = root
  -- A fresh open carries no buffer expectation, so a panel the user
  -- killed earlier does not make this run drop its own first result.
  state.buffer = nil
  -- The generation reserved at the command, NOT a fresh one --- and the
  -- destination captured there, for the same reason. This callback runs
  -- after `git rev-parse` has exited, so capturing here would read the
  -- ambient frontend a round trip late and reintroduce exactly the
  -- misrouting the capture exists to close.
  start_status(root, nil, false, request.generation, request.dest)
end

--- Open (or re-open) `*git-status*` for the repository containing the
--- active file.
function pmacs.git.status()
  if not git_enabled() then
    pmacs.editor.set_status("git: disabled by the `git.enabled` setting")
    return
  end
  local dir = active_directory()
  if not dir then
    pmacs.editor.set_status("git: no directory to resolve a repository from")
    return
  end
  -- Reserved AFTER the early returns and BEFORE the spawn: an
  -- invocation that starts no work must not invalidate one that is
  -- already in flight, and an invocation that does start work must own
  -- the newest generation from that moment on.
  -- Captured alongside the generation, at the same moment and for the
  -- same reason: this is the last instant at which "the frontend the
  -- user asked from" is knowable without guessing.
  local request = {
    generation = status_requests.reserve(),
    dir = dir,
    dest = pmacs.window.capture_destination(),
  }
  -- The root rule (Q#G-2): ask git, and let a non-zero exit BE the
  -- "not a repository" answer. `-C <dir>` with no root of our own.
  run_git("git rev-parse",
    "resolving which Git repository contains " .. dir,
    nil, { "-C", dir, "rev-parse", "--show-toplevel" },
    function(res) pmacs.git._deliver_root(request, res) end)
end

pmacs.command.define {
  name = "git.status",
  description = "Show the working tree's Git status in a *git-status* panel.",
  fn = pmacs.git.status,
}

-- ---------------------------------------------------------------------
-- The diff gesture (Q#G-7)
-- ---------------------------------------------------------------------
--
-- RET visits the file --- the behaviour a list of files should have ---
-- so the diff needs its own key, and `d` is it.
--
-- What `d` shows answers the lane's own question, "what have I
-- changed?", against HEAD. A porcelain-v2 row carries an XY pair (X
-- staged, Y unstaged) and the three plausible diffs answer three
-- different questions: `git diff` shows only Y, `--cached` only X, and
-- neither shows an untracked file at all. One view of the TOTAL change
-- is right for reading; splitting X from Y is a staging UI, which is
-- Stage 3.
--
-- `--no-color` because this renders as plain text and a user with
-- `color.ui = always` would otherwise get escape sequences in a buffer
-- with no ANSI parser behind it.

local SPLIT_HEADER =
  "no commits yet --- split view: staged (index) above, unstaged (worktree) below"

-- True when a `2` record is a COPY rather than a rename.
--
-- Read from `score`, not from `row.x`. The `<Xscore>` field names
-- rename-vs-copy whichever side detected the change, while `X` carries
-- the letter only for an index-side one --- a worktree-side detection
-- puts it in `Y` and leaves `X` a `.`. Absent or malformed, this says
-- "not a copy", so the header falls back to the commoner of the two
-- rather than to a claim it cannot support.
local function is_copy(row)
  return (row.score or ""):sub(1, 1) == "C"
end

-- The invocations `d` runs for `row`, as
-- `{ { label = string|nil, args = {...}, no_index = bool }, ... }`,
-- plus the header describing what the result shows.
local function diff_plan(row, unborn)
  local path = row.path
  if row.kind == "untracked" or row.kind == "ignored" then
    -- A normal diff shows NOTHING for an untracked file. Without this
    -- case `d` is silently dead on the rows a user is most likely to
    -- press it on.
    return {
      header = "untracked --- shown against /dev/null",
      steps = { { args = { "diff", "--no-color", "--no-index", "--", "/dev/null", path },
                  no_index = true } },
    }
  end
  if not unborn then
    if row.kind == "rename" and row.orig then
      -- Both paths, which is what lets rename detection render this as
      -- a rename rather than an unrelated add plus delete. Identical for
      -- a copy, which is why the two share a `kind` --- only the WORD
      -- differs, because a copy left the origin where it was and saying
      -- "renamed" of it states a different fact about the user's tree.
      return {
        header = string.format("against HEAD (%s from %s)",
          is_copy(row) and "copied" or "renamed",
          pmacs.git.display_path(row.orig)),
        steps = { { args = { "diff", "--no-color", "HEAD", "--", row.orig, path } } },
      }
    end
    return {
      header = "against HEAD",
      steps = { { args = { "diff", "--no-color", "HEAD", "--", path } } },
    }
  end
  -- Unborn HEAD: there is nothing to total AGAINST, so the split
  -- appears here and only here. `AM` and `AD` carry BOTH states at
  -- once, which is exactly the gap: `--cached` alone loses the worktree
  -- delta and plain `git diff` alone loses the staged base.
  local staged = row.x ~= "." and row.x ~= " "
  local unstaged = row.y ~= "." and row.y ~= " "
  local cached = { label = "staged (index)",
                   args = { "diff", "--no-color", "--cached", "--", path } }
  local worktree = { label = "unstaged (worktree)",
                     args = { "diff", "--no-color", "--", path } }
  if staged and unstaged then
    return { header = SPLIT_HEADER, steps = { cached, worktree } }
  end
  if staged then
    return { header = "no commits yet --- staged (index) only", steps = { cached } }
  end
  return { header = "no commits yet --- unstaged (worktree) only", steps = { worktree } }
end

local function diff_step_ok(step, res)
  if not res.ok then return false end
  -- `--no-index` implies `--exit-code`: exit 1 means it SUCCESSFULLY
  -- found differences, which is the whole point of running it. Under a
  -- plain "non-zero is failure" predicate every untracked diff would
  -- render a failure row instead of the diff it just produced. The
  -- asymmetry is confined to this invocation.
  if step.no_index then return (res.code or 0) <= 1 end
  return (res.code or 0) == 0
end

-- Ownership is the HANDLE this module holds, never a name match ---
-- listview's Q#GB13 rule and dired's F7 rule, for the same reason.
-- `pmacs.buffer.create` takes any caller-chosen name, so a user may
-- already have a buffer called `*git-diff*`; adopting it would clobber
-- their bytes and then lock the rope. A fresh create leaves theirs
-- untouched, and this module writes only to the handle it made.
local function show_diff_buffer(title, body)
  local buf = state.diff_buffer
  local live = buf ~= nil and select(2, pcall(buf.is_valid, buf)) == true
  if not live then
    buf = pmacs.buffer.create(DIFF_BUFFER)
    pmacs.buffer.add_intercept(buf, function()
      error(DIFF_BUFFER .. " is read-only")
    end)
    pmacs.buffer.set_round_trip_input(buf, true)
    state.diff_buffer = buf
  end
  pmacs.buffer.set_generated_contents(buf, title .. "\n\n" .. body)
  -- The DOCUMENT target, so the status panel it was invoked from stays
  -- visible beside it.
  pcall(pmacs.window.display, buf, { select = true })
end

-- Start the request's next step, or render what the finished ones
-- produced.
--
-- Everything this needs lives on `request`, captured at the keypress:
-- the diff ticket, the row, the plan, and the ROOT. `state.root` is
-- deliberately not read anywhere in here. An unborn `AM`/`AD` row
-- produces a TWO-STEP plan, and `state.root` is module-level mutable
-- state that a concurrent `git.status` against another repository
-- reassigns from its own root-resolution callback --- so reading it per
-- step would let one plan's second step run in a different repository
-- than its first, carrying the first repository's path.
local function advance_diff(request)
  request.index = request.index + 1
  local step = request.plan.steps[request.index]
  if not step then
    local body = table.concat(request.pieces, "\n")
    if body:gsub("%s", "") == "" then
      body = "(no differences)"
    end
    -- The DOCUMENT profile, and the contrast with the status channel is
    -- the whole of Q#DC-2: `*git-diff*` REPLACES a document window, so
    -- every stale-intent check applies. If the window the `d` was
    -- pressed from now holds a different buffer, this diff is answering
    -- a question about a view the user has already left, and the commit
    -- is refused rather than allowed to overwrite it.
    commit_ui(request.dest, "document", function()
      show_diff_buffer(string.format("git diff --- %s\n%s",
        pmacs.git.display_path(request.row.path), request.plan.header), body)
    end)
    return
  end
  run_git("git diff",
    "diffing " .. pmacs.git.display_path(request.row.path) .. " against HEAD for *git-diff*",
    request.root, step.args,
    function(res) pmacs.git._deliver_diff(request, step, res) end)
end

--- Deliver a completed diff STEP for the plan in `request`.
---
--- Exposed for the same reason `_deliver_status` and `_deliver_root`
--- are: the contract is about completions arriving in an order the
--- CALLER did not choose, and no arrangement of real subprocess timing
--- can guarantee that two `git diff` runs finish in a chosen order.
function pmacs.git._deliver_diff(request, step, res)
  -- Superseded by a newer `d`: discard BEFORE ANY EFFECT. `*git-diff*`
  -- is a singleton buffer, so without this a slow first request
  -- overwrites a fast second one --- the newest invocation loses to the
  -- slowest subprocess, which is the same defect the status channel had.
  --
  -- This is the ONE place a diff plan re-enters from a continuation, so
  -- one check covers all of it: no further spawn (`advance_diff` is
  -- never reached), no buffer write, and no status message either, since
  -- a status line from a replaced invocation is as wrong as a buffer
  -- from one. The in-flight process is not terminated --- reaping is
  -- `process.forget`'s job, exactly as on the status channel.
  if not diff_requests.is_current(request.generation) then return end
  if not diff_step_ok(step, res) then
    local reason = failure_reason(res)
    -- The failure render is a render: same destination, same profile,
    -- same refusal. Announcing a diff failure into whatever frontend
    -- happens to be active would be the identical misrouting as
    -- announcing a success there.
    commit_ui(request.dest, "document", function()
      show_diff_buffer(string.format("git diff --- %s",
        pmacs.git.display_path(request.row.path)), reason)
      pmacs.editor.set_status("git diff: " .. reason)
    end)
    return
  end
  local text = utf8_clean(res.stdout)
  if step.label then
    request.pieces[#request.pieces + 1] = string.format("=== %s ===\n%s", step.label,
      text ~= "" and text or "(no changes)\n")
  else
    request.pieces[#request.pieces + 1] = text
  end
  advance_diff(request)
end

-- Run `plan`'s steps in order under an ALREADY-RESERVED diff ticket,
-- then render. `generation` is a parameter for the same reason
-- `start_status`'s is: it belongs to the keypress, not to this call.
local function run_diff_plan(row, plan, root, generation, dest)
  advance_diff {
    generation = generation, row = row, plan = plan, root = root,
    pieces = {}, index = 0, dest = dest,
  }
end

pmacs.command.define {
  name = "git.diff-file",
  description = "Show the diff for the file under the cursor in *git-status*.",
  fn = function()
    -- Bound buffer-locally on the panel, so `d` can only reach this
    -- there; the identity check is for the `M-x` path. Compared against
    -- the CAPTURED handle, never a name lookup: listview disambiguates
    -- a collision to `<2>`, so the name is not the identity.
    local active = pmacs.window.buffer()
    if not (state.buffer and active and active == state.buffer) then
      pmacs.editor.set_status("git: no *git-status* row here")
      return
    end
    local row = state.display[pmacs.editor.cursor_line()]
    if not (type(row) == "table" and row.path) then
      pmacs.editor.set_status("git: no file on this line")
      return
    end
    if not pmacs.git.is_text(row.path)
      or (row.orig and not pmacs.git.is_text(row.orig)) then
      refuse_unrepresentable(row)
      return
    end
    if not git_enabled() then
      pmacs.editor.set_status("git: disabled by the `git.enabled` setting")
      return
    end
    -- Everything the plan runs on is captured HERE, at the keypress, and
    -- threaded through every step: the ticket, reserved AFTER the early
    -- returns above so a `d` that starts no work cannot supersede one
    -- that is already in flight; the root; the unborn flag; and the
    -- DESTINATION. The first three describe the repository the user is
    -- looking at right now and are replaced wholesale by a `git.status`
    -- against another one; the fourth describes the window they are
    -- looking at it IN, which nothing in this module replaces and
    -- nothing outside it announces.
    run_diff_plan(row, diff_plan(row, (state.branch or {}).unborn == true),
      state.root, diff_requests.reserve(), pmacs.window.capture_destination())
  end,
}
