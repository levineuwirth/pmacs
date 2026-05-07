-- pmacs-magit/gestures.lua --- Stage/unstage/commit/push/branch (T M8.7).
--
-- The magit-class gesture surface: keybindings on the magit buffer
-- that run the corresponding git command on the section / item
-- under cursor. Multi-step gestures (commit, push, branch create)
-- compose with `pmacs.minibuffer.read` for user input; single-step
-- gestures (stage, unstage, branch checkout-from-list) run git
-- directly.
--
-- Public surface (one function per gesture, all run inside a
-- pmacs.async coroutine; the handle's `refresh_pending` is set
-- around git invocations so the polling loop doesn't double-fire):
--
--   item_at_cursor(handle)        -> { kind, path } | nil
--   stage(handle)                 -> nil
--   unstage(handle)               -> nil
--   commit(handle)                -> nil  (uses minibuffer)
--   push(handle)                  -> nil  (uses minibuffer)
--   branch_create(handle)         -> nil  (uses minibuffer)
--   branch_switch(handle)         -> nil  (uses minibuffer w/ candidates)
--
-- Each gesture re-fetches status after a successful git invocation
-- so the buffer reflects the new state before the next polling
-- tick. Failed git invocations surface stderr to set_status; the
-- buffer's previous state stays.

local M = {}

local status = require("pmacs-magit.status")

-- ---------------------------------------------------------------------------
-- Item under cursor
-- ---------------------------------------------------------------------------
--
-- The magit buffer's body lines for the working / staged sections
-- look like:
--
--   "M README.md"      (modified, indexed)
--   "?? new.txt"       (untracked)
--   "AM combined.txt"  (added then modified)
--
-- That is: "<status-bytes> <path>". For non-file sections (log,
-- branches, stashes) item_at_cursor returns just the section id
-- with kind = "section"; the gesture handler decides whether to act
-- (e.g., `b c` works in any section because it doesn't operate on
-- a specific item).

-- Read the text of a 0-indexed visible-buffer line. Walks the rope
-- once to find the Nth newline boundary; returns the slice between
-- it and the next. Returns nil if the line index is out of range.
local function read_visible_line(handle, line)
  local total_len = handle.visible:len()
  local raw = handle.visible:slice(0, total_len)
  local cur_line = 0
  local pos = 1
  while pos <= #raw + 1 do
    local nl = raw:find("\n", pos, true)
    local end_pos = nl or (#raw + 1)
    if cur_line == line then
      return raw:sub(pos, end_pos - 1)
    end
    if nl == nil then return nil end
    pos = nl + 1
    cur_line = cur_line + 1
  end
  return nil
end

-- Parse a body line into { status_code, path }, or nil if the line
-- doesn't match the expected shape (e.g., a placeholder like
-- "(no working-tree changes)" or a header line).
local function parse_body_line(line_text)
  if line_text == nil then return nil end
  -- Body lines are indented by 2 spaces (one indent level past the
  -- top-level header). Strip leading spaces.
  local trimmed = line_text:match("^%s*(.+)$")
  if trimmed == nil then return nil end
  if trimmed:sub(1, 1) == "(" then return nil end  -- placeholder
  -- "<status-code> <path>" or "?? <path>" or "AM <path>".
  local status_code, path = trimmed:match("^(%S+)%s+(.+)$")
  if status_code == nil or path == nil then return nil end
  return { status_code = status_code, path = path }
end

-- Resolve the section + item under the cursor.
function M.item_at_cursor(handle)
  if not handle.projection then return nil end
  local line = pmacs.editor.cursor_line()
  local section_id = handle.projection.fold_targets[line]
  if section_id == nil then return nil end
  -- Header line of the section -> kind = "section_header".
  -- Body line -> parse.
  local section_lines = {}
  for ln, sid in pairs(handle.projection.fold_targets) do
    if sid == section_id and (section_lines.first == nil or ln < section_lines.first) then
      section_lines.first = ln
    end
  end
  if line == section_lines.first then
    return { kind = "section_header", section = section_id }
  end
  -- Body line. The line text comes from the buffer.
  local line_text = read_visible_line(handle, line)
  local parsed = parse_body_line(line_text)
  if parsed == nil then
    return { kind = "placeholder", section = section_id }
  end
  return {
    kind = "item",
    section = section_id,
    status_code = parsed.status_code,
    path = parsed.path,
  }
end

-- ---------------------------------------------------------------------------
-- Run a git command, then refresh the buffer
-- ---------------------------------------------------------------------------
--
-- Every gesture funnels through this wrapper so the
-- run-git-then-refresh dance is a single shape. `cmd` is the args
-- array (e.g., {"add", "README.md"}); the implicit "git" is added by
-- status.run_git. On non-zero exit, surface stderr via
-- pmacs.editor.set_status; on success, call status.fetch_sections
-- and update_spec to repaint with the new state.
--
-- Returns true on success, false on failure (stderr already
-- surfaced).

local function run_and_refresh(handle, args, label, update_spec_fn)
  if handle.refresh_pending then
    -- Coalesce: skip if the polling loop is mid-refresh. The user
    -- can retry a beat later.
    if pmacs.editor and pmacs.editor.set_status then
      pmacs.editor.set_status(
        "pmacs-magit." .. label .. ": refresh in progress, retry shortly"
      )
    end
    return false
  end
  handle.refresh_pending = true
  local result = status.run_git(args, { cwd = handle.repo_root })
  if not result.ok then
    handle.refresh_pending = false
    if pmacs.editor and pmacs.editor.set_status then
      local trimmed = (result.stderr or ""):match("^%s*(.-)%s*$") or ""
      pmacs.editor.set_status(
        "pmacs-magit." .. label .. ": git failed (exit " ..
        tostring(result.exit_code) .. "): " .. trimmed
      )
    end
    return false
  end
  -- git succeeded; re-fetch and repaint.
  local result = status.fetch_sections(handle.repo_root)
  if handle.alive then
    update_spec_fn(handle, result.spec)
    handle.branches_snapshot =
      (result.parsed.branches and result.parsed.branches.all) or {}
  end
  handle.refresh_pending = false
  return true
end

-- ---------------------------------------------------------------------------
-- Stage / unstage
-- ---------------------------------------------------------------------------
--
-- Stage (`s` binding): `git add <path>` for the file under cursor.
-- Allowed only when cursor is on a "working" section item; outside
-- that, surfaces a clear "no-op" message via the modeline rather
-- than silently doing nothing.

function M.stage(handle, update_spec_fn)
  local item = M.item_at_cursor(handle)
  if item == nil or item.kind ~= "item" or item.section ~= "working" then
    if pmacs.editor and pmacs.editor.set_status then
      pmacs.editor.set_status(
        "pmacs-magit.stage: cursor must be on a working-tree-change item"
      )
    end
    return
  end
  run_and_refresh(handle, { "add", "--", item.path }, "stage", update_spec_fn)
end

function M.unstage(handle, update_spec_fn)
  local item = M.item_at_cursor(handle)
  if item == nil or item.kind ~= "item" or item.section ~= "staged" then
    if pmacs.editor and pmacs.editor.set_status then
      pmacs.editor.set_status(
        "pmacs-magit.unstage: cursor must be on a staged item"
      )
    end
    return
  end
  run_and_refresh(
    handle,
    { "restore", "--staged", "--", item.path },
    "unstage",
    update_spec_fn
  )
end

-- ---------------------------------------------------------------------------
-- Commit (composes in a message buffer, magit-style)
-- ---------------------------------------------------------------------------
--
-- The M8.7 spec requires "commit composes a message buffer", so a
-- one-line minibuffer prompt isn't enough --- multi-line subject +
-- body editing wants a proper buffer. The flow:
--
--   1. M.commit creates `*magit-commit*` (a fresh buffer), pre-
--      populates a scaffold (blank message line, then comment
--      lines explaining the keybindings + branch + repo), binds
--      `C-c C-c` (submit) and `C-c C-k` (cancel) on it, switches
--      the active window to it. Stores `handle.commit_session`
--      so the submit/cancel commands can find their way home.
--
--   2. User types a message. Multi-line is fine; comment lines
--      starting with `#` are stripped at submit time (matching
--      git's own COMMIT_EDITMSG convention).
--
--   3. C-c C-c (M.commit_submit) extracts the cleaned message,
--      writes it to a tempfile, runs `git commit -F <path>`,
--      removes the tempfile, refreshes, kills the commit buffer
--      and switches the window back to the magit buffer.
--
--   4. C-c C-k (M.commit_cancel) just kills the buffer and
--      switches back; no git invocation.

local function commit_scaffold(handle)
  local lines = {
    "",  -- empty first line --- the user's subject lands here
    "",
    "# Please enter the commit message. Lines starting with '#' are",
    "# ignored. C-c C-c finishes the commit; C-c C-k cancels.",
    "#",
    "# Branch: " .. (handle.current_branch_snapshot or "?"),
    "# Repo:   " .. (handle.repo_root or "?"),
  }
  return table.concat(lines, "\n")
end

local function find_commit_session(handles_arr)
  local active = pmacs.window.buffer()
  for _, h in ipairs(handles_arr) do
    if h.commit_session and h.commit_session.buffer == active then
      return h
    end
  end
  return nil
end

-- The handles_arr argument is the package's handle list, threaded
-- through from init.lua. Lets the commit-submit / commit-cancel
-- commands resolve "which magit handle owns this *magit-commit*
-- buffer" without reaching into init.lua's locals.
function M.find_commit_session(handles_arr)
  return find_commit_session(handles_arr)
end

function M.commit(handle, _update_spec_fn)
  -- Disallow re-entering commit while one is already open --- the
  -- handle.commit_session field points at the existing buffer, and
  -- creating a second one without cleanup would leak.
  if handle.commit_session then
    if pmacs.editor and pmacs.editor.set_status then
      pmacs.editor.set_status(
        "pmacs-magit.commit: a commit-message buffer is already open; " ..
        "switch to it (or use C-c C-k there) before starting another"
      )
    end
    return
  end
  local buf = pmacs.buffer.create("*magit-commit*")
  -- Paint the scaffold. The buffer is freshly created with no
  -- intercepts, so a direct :replace works. (No painting flag
  -- needed --- nobody else is watching writes to this buffer.)
  buf:replace(0, buf:len(), commit_scaffold(handle))
  handle.commit_session = {
    buffer = buf,
    return_to = handle.visible,
  }
  pmacs.keymap.bind {
    scope = "buffer", buffer = buf, sequence = "C-c C-c",
    command = "pmacs-magit.commit-submit",
  }
  pmacs.keymap.bind {
    scope = "buffer", buffer = buf, sequence = "C-c C-k",
    command = "pmacs-magit.commit-cancel",
  }
  pmacs.window.switch_buffer(buf)
end

-- Read the commit message out of the buffer, strip lines starting
-- with `#`, drop trailing empty lines. Returns the cleaned message
-- string (empty string if there's no real content).
local function extract_message(buf)
  local raw = buf:slice(0, buf:len())
  local out = {}
  -- gmatch "[^\n]*" captures each line (including the trailing
  -- empty string from a final newline). We filter comment lines
  -- and trim trailing empties at the end.
  for line in (raw .. "\n"):gmatch("([^\n]*)\n") do
    if line:sub(1, 1) ~= "#" then
      out[#out + 1] = line
    end
  end
  while #out > 0 and out[#out] == "" do
    out[#out] = nil
  end
  return table.concat(out, "\n")
end

function M.commit_submit(handle, update_spec_fn)
  local session = handle.commit_session
  if session == nil then
    if pmacs.editor and pmacs.editor.set_status then
      pmacs.editor.set_status(
        "pmacs-magit.commit-submit: no commit-message buffer is open"
      )
    end
    return
  end
  local message = extract_message(session.buffer)
  if message == "" then
    if pmacs.editor and pmacs.editor.set_status then
      pmacs.editor.set_status(
        "pmacs-magit.commit-submit: empty message; type a subject line " ..
        "or use C-c C-k to cancel"
      )
    end
    return
  end

  -- Tear down the commit session BEFORE the async work fires. If
  -- we left it set, a second `c` press during the in-flight commit
  -- would fall into the "already open" branch and surprise the user.
  local return_to = session.return_to
  local commit_buf = session.buffer
  handle.commit_session = nil
  pmacs.window.switch_buffer(return_to)
  if pmacs.buffer.kill then
    pcall(pmacs.buffer.kill, commit_buf)
  end

  pmacs.async(function()
    -- Write the message to a tempfile and run `git commit -F`.
    -- We use a tempfile rather than `git commit -F -` (stdin)
    -- because pmacs.process doesn't expose a "close stdin"
    -- primitive that would let `git` see EOF cleanly --- a
    -- write_stdin then immediate exit is racy. The tempfile
    -- shape is what `git`'s own editor-driven flow uses, so
    -- behavior matches.
    local tmpfile = os.tmpname()
    local f, err = io.open(tmpfile, "w")
    if f == nil then
      if pmacs.editor and pmacs.editor.set_status then
        pmacs.editor.set_status(
          "pmacs-magit.commit: tempfile open failed: " .. tostring(err)
        )
      end
      return
    end
    f:write(message)
    f:close()
    run_and_refresh(
      handle,
      { "commit", "-F", tmpfile },
      "commit",
      update_spec_fn
    )
    pcall(os.remove, tmpfile)
  end)
end

function M.commit_cancel(handle)
  local session = handle.commit_session
  if session == nil then
    if pmacs.editor and pmacs.editor.set_status then
      pmacs.editor.set_status(
        "pmacs-magit.commit-cancel: no commit-message buffer is open"
      )
    end
    return
  end
  local return_to = session.return_to
  local commit_buf = session.buffer
  handle.commit_session = nil
  pmacs.window.switch_buffer(return_to)
  if pmacs.buffer.kill then
    pcall(pmacs.buffer.kill, commit_buf)
  end
  if pmacs.editor and pmacs.editor.set_status then
    pmacs.editor.set_status("pmacs-magit.commit: cancelled")
  end
end

-- ---------------------------------------------------------------------------
-- Push (uses minibuffer; defaults to "origin")
-- ---------------------------------------------------------------------------
--
-- Push prompts for the remote name with "origin" as the default
-- (most common case). The current branch is implied by the active
-- branch in the snapshot.

function M.push(handle, update_spec_fn)
  pmacs.minibuffer.read {
    prompt = "Push to remote: ",
    initial = "origin",
    history = "pmacs-magit.push",
    on_accept = function(remote)
      if remote == nil or remote == "" then
        if pmacs.editor and pmacs.editor.set_status then
          pmacs.editor.set_status("pmacs-magit.push: empty remote; aborted")
        end
        return
      end
      -- `-u <remote> HEAD` is the v0.1 default:
      --   * `HEAD` resolves to the current branch, so `git push`
      --     pushes that branch (rather than relying on
      --     `push.default`, which can be `simple` / `current` /
      --     `upstream` and behaves differently on first push).
      --   * `-u` sets upstream tracking. On first push this is the
      --     critical bit: `git push <remote>` alone fails with
      --     "current branch has no upstream branch" on a fresh
      --     remote-add. With `-u`, the first push sets tracking
      --     and subsequent pushes are no-ops on the upstream side.
      -- A future v0.2 magit-class can split "push without -u" into
      -- a separate gesture (e.g., `P P`); v0.1 prioritizes the
      -- spec-stated "push to a configured remote works" case.
      pmacs.async(function()
        run_and_refresh(
          handle,
          { "push", "-u", remote, "HEAD" },
          "push",
          update_spec_fn
        )
      end)
    end,
    on_cancel = function()
      if pmacs.editor and pmacs.editor.set_status then
        pmacs.editor.set_status("pmacs-magit.push: cancelled")
      end
    end,
  }
end

-- ---------------------------------------------------------------------------
-- Branch create / switch
-- ---------------------------------------------------------------------------
--
-- branch_create prompts for a new branch name and creates+switches
-- to it (`git checkout -b <name>`).
-- branch_switch prompts with completion candidates from the current
-- branch list and switches (`git checkout <name>`).

function M.branch_create(handle, update_spec_fn)
  pmacs.minibuffer.read {
    prompt = "Create branch: ",
    initial = "",
    history = "pmacs-magit.branch-create",
    on_accept = function(name)
      if name == nil or name == "" then
        if pmacs.editor and pmacs.editor.set_status then
          pmacs.editor.set_status("pmacs-magit.branch-create: empty name; aborted")
        end
        return
      end
      pmacs.async(function()
        run_and_refresh(
          handle,
          { "checkout", "-b", name },
          "branch-create",
          update_spec_fn
        )
      end)
    end,
    on_cancel = function()
      if pmacs.editor and pmacs.editor.set_status then
        pmacs.editor.set_status("pmacs-magit.branch-create: cancelled")
      end
    end,
  }
end

-- For branch-switch, we surface the existing branch list as the
-- minibuffer's completion source. The list comes from the latest
-- snapshot via the parsed branches result we cached on the handle
-- (init.lua's update_spec stores it for this purpose).

function M.branch_switch(handle, update_spec_fn)
  pmacs.minibuffer.read {
    prompt = "Switch to branch: ",
    initial = "",
    history = "pmacs-magit.branch-switch",
    -- The minibuffer's Custom completion source is a function
    -- called with no args returning the candidate list. We pull
    -- from handle.branches_snapshot (init.lua's update_spec sets
    -- this on every refresh).
    source = function() return handle.branches_snapshot or {} end,
    on_accept = function(name)
      if name == nil or name == "" then
        if pmacs.editor and pmacs.editor.set_status then
          pmacs.editor.set_status("pmacs-magit.branch-switch: empty name; aborted")
        end
        return
      end
      pmacs.async(function()
        run_and_refresh(
          handle,
          { "checkout", name },
          "branch-switch",
          update_spec_fn
        )
      end)
    end,
    on_cancel = function()
      if pmacs.editor and pmacs.editor.set_status then
        pmacs.editor.set_status("pmacs-magit.branch-switch: cancelled")
      end
    end,
  }
end

return M
