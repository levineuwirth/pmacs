-- pmacs-magit/init.lua --- Foldable section view (T M8.5).
--
-- The magit-class entry of M8's three universality-proof packages.
-- Validates the *selective rendering* shape: a buffer holds the
-- full rendered text; a separate visible buffer is the projection
-- the user navigates, rebuilt on each fold operation. The fold
-- state lives in a Lua table per handle, not in any rope.
--
-- This v0.1 covers the read-only foldable-section primitive.
-- Git status integration (M8.6) and gestures (M8.7) build on it.
--
-- Public surface:
--
--   local magit = require("pmacs-magit")
--   magit.open {
--     id = "root",
--     title = "Demo",
--     children = {
--       { id = "a", title = "Section A", body = "line1\nline2" },
--       { id = "b", title = "Section B", body = "line3" },
--     },
--   }
--
--   -- inside a magit-section buffer:
--   --   TAB          -> toggle fold of section under cursor
--   --   M-x pmacs-magit.fold-all
--   --   M-x pmacs-magit.unfold-all
--
-- Architecture (T M8.5 Plan A --- two-buffer projection):
--
--   * source buffer: canonical rope, held in the handle. Its rope is
--     the full rendered text (every section, header + body, regardless
--     of fold state). Folding never rewrites this buffer. pmacs v0.1
--     has no hidden-buffer API, so the source buffer is protected by a
--     read-only intercept rather than relying on a naming convention.
--
--   * visible buffer: what the user sees. Rebuilt
--     on every fold operation as the projection of (flat) through
--     (fold_state). The user navigates this buffer directly;
--     C-n / C-p / etc. naturally skip folded content because it
--     isn't in the visible buffer at all.
--
--   * fold_state: Lua table mapping section_id to "collapsed" or
--     "expanded" (default). Lives in the handle; survives any
--     repaint of the visible buffer (which is the M8.5 acceptance
--     bullet "folding state survives buffer redraw and view
--     repaint").
--
--   * intercept on the visible buffer rejects user edits. The
--     package's own paint operations bypass the intercept via the
--     painting flag (same shape as dired-class, with the same
--     pcall guard --- M8.4 audit finding 2).
--
--   * cursor reseat: every repaint captures the section under
--     cursor first, repaints, then moves the cursor to the same
--     section's new header line. Without this the engine's stale
--     byte-offset behavior (documented in
--     builtin/commands/default.lua:308) leaves the cursor at an
--     arbitrary spot after fold-all / unfold-all / a body-line
--     toggle.

local fold = require("pmacs-magit.fold")
local status = require("pmacs-magit.status")
local gestures = require("pmacs-magit.gestures")

local M = {}

-- ---------------------------------------------------------------------------
-- Per-buffer state
-- ---------------------------------------------------------------------------

local handles = {}

local function cleanup_handle(handle)
  -- Flip the alive flag first so the M8.6 polling loop exits at its
  -- next wake-up rather than running one more refresh against a
  -- dead handle.
  handle.alive = false
  if handle.intercept then
    pmacs.buffer.remove_intercept(handle.intercept)
    handle.intercept = nil
  end
  if handle.source_intercept then
    pmacs.buffer.remove_intercept(handle.source_intercept)
    handle.source_intercept = nil
  end
  if handle.source then
    if pmacs.buffer.kill then
      pcall(pmacs.buffer.kill, handle.source)
    end
    handle.source = nil
  end
  -- M8.7 commit-message buffer (if one's open). Killing it leaves
  -- whatever was there as scratch text in the user's mental model;
  -- since the magit handle this buffer was attached to is going
  -- away, there's nothing to do with the message anyway.
  if handle.commit_session then
    if pmacs.buffer.kill then
      pcall(pmacs.buffer.kill, handle.commit_session.buffer)
    end
    handle.commit_session = nil
  end
end

local function find_handle(visible_buf)
  -- Same shape as dired-class: probe :is_valid() so a removed
  -- magit buffer doesn't keep its handle alive forever.
  -- (M8.4 finding 3.)
  local live = {}
  local found = nil
  for _, h in ipairs(handles) do
    local ok, valid = pcall(h.visible.is_valid, h.visible)
    if ok and valid then
      live[#live + 1] = h
      if h.visible == visible_buf then
        found = h
      end
    else
      cleanup_handle(h)
    end
  end
  handles = live
  return found
end

local function active_handle()
  return find_handle(pmacs.window.buffer())
end

-- Locate the magit handle whose `*magit-commit*` buffer is currently
-- active. Used by the `commit-submit` / `commit-cancel` commands,
-- which are bound on the commit buffer (so `active_handle()` returns
-- nil --- the active buffer isn't a magit *visible* buffer). Walks
-- the handle list since the commit buffer's identity isn't part of
-- the keymap-stack binding context.
local function commit_session_handle()
  local active = pmacs.window.buffer()
  for _, h in ipairs(handles) do
    if h.commit_session and h.commit_session.buffer == active then
      return h
    end
  end
  return nil
end

-- ---------------------------------------------------------------------------
-- Painting
-- ---------------------------------------------------------------------------
--
-- Paint a string into a buffer with the painting-flag bypass. The
-- intercept attached to the visible buffer rejects all user edits
-- and lets package writes pass; the flag is the discriminator.
-- The full op runs inside pcall so the flag is guaranteed to clear
-- even if `:replace` (or `:len`) raises --- same shape as
-- dired-class's `paint`, M8.4 audit finding 2.

local function paint_buffer(handle, buf, text)
  handle.painting = true
  local ok, err = pcall(function()
    buf:replace(0, buf:len(), text)
  end)
  handle.painting = false
  if not ok then error(err) end
end

-- Return the cursor's current line if and only if the active window
-- is the visible buffer of `handle`. Otherwise nil --- a repaint
-- triggered while the user is in some other buffer must not perturb
-- their cursor there.
local function visible_cursor_line(handle)
  if pmacs.window.buffer() == handle.visible then
    return pmacs.editor.cursor_line()
  end
  return nil
end

-- Move the cursor in the active window to `target_line` (0-indexed)
-- using only the public `move_up` / `move_down` primitives. After a
-- wholesale `:replace`, the engine leaves the cursor at a stale
-- byte offset (documented in builtin/commands/default.lua:308); we
-- can't rely on the post-replace line being any particular value.
-- The most robust shape is: walk up to bottom-out at line 0 (the
-- move_up no-op signals BOB), then walk down to target.
local function move_cursor_to_line(target_line)
  -- Walk up until move_up no longer changes the line (we've hit
  -- the start of the buffer). Cap with a generous step counter so
  -- a buggy engine can't loop forever.
  local guard = 0
  local prev = pmacs.editor.cursor_line()
  while guard < 100000 do
    pmacs.editor.move_up()
    local now = pmacs.editor.cursor_line()
    if now == prev then break end
    prev = now
    guard = guard + 1
  end
  -- Now at line 0. Walk down target_line steps.
  for _ = 1, target_line do
    pmacs.editor.move_down()
  end
end

local function repaint_visible(handle)
  -- Capture the section the cursor is on before we repaint, so we
  -- can move back to that section's header in the new projection.
  -- A nil here just means the user isn't currently in this view; we
  -- skip cursor reseat.
  local cursor_line_before = visible_cursor_line(handle)
  local section_id_before = nil
  if cursor_line_before ~= nil and handle.projection then
    section_id_before = fold.section_at(handle.projection, cursor_line_before)
  end

  local proj = fold.render_visible(handle.flat, handle.fold_state)
  handle.projection = proj
  paint_buffer(handle, handle.visible, proj.text)

  -- Build a section-id -> first-visible-line lookup from the new
  -- projection. fold_targets entries are dense across visible
  -- lines (header + body lines all map back to a section); we
  -- want the smallest line for each section id, which is its
  -- header line under the renderer's contract.
  local section_lines = {}
  for line, sid in pairs(proj.fold_targets) do
    if section_lines[sid] == nil or line < section_lines[sid] then
      section_lines[sid] = line
    end
  end

  -- Reseat cursor on the same section if possible. If the section
  -- got hidden (its parent collapsed), walk up the parent chain
  -- until we find an ancestor whose header is in the new
  -- projection. handle.parent_of is built once at open() time
  -- from the flat parse.
  if section_id_before ~= nil and visible_cursor_line(handle) ~= nil then
    local target_id = section_id_before
    while target_id ~= nil and section_lines[target_id] == nil do
      target_id = handle.parent_of[target_id]
    end
    if target_id ~= nil then
      local target_line = section_lines[target_id]
      if target_line ~= nil then
        move_cursor_to_line(target_line)
      end
    end
  end
end

-- ---------------------------------------------------------------------------
-- Live update of an existing magit buffer
-- ---------------------------------------------------------------------------
--
-- M8.5 ships `M.open(spec)` for one-shot section views. M8.6 needs
-- to *update* the spec while the buffer is open: each git refresh
-- produces a new spec the package paints into the existing visible
-- (and source) buffer.
--
-- update_spec preserves fold_state across the swap: section IDs are
-- stable across refreshes (e.g., always `"working"`, `"staged"`,
-- ..., never `"working-3"`-style IDs that drift with content), so
-- a section the user collapsed before the refresh stays collapsed
-- after. Cursor reseat happens through repaint_visible's existing
-- machinery --- if the user was on Section X before refresh, they
-- end up on Section X's new header line after.

local function update_spec(handle, new_spec)
  local flat = fold.parse(new_spec)
  if #flat == 0 then return end
  handle.flat = flat
  local sr = fold.render_source(flat)
  handle.source_text = sr.text
  handle.source_line_index = sr.line_index
  -- Rebuild parent_of: structurally stable for magit-status today,
  -- but we don't *rely* on that --- a future caller of update_spec
  -- might legitimately swap the section tree, and the cursor-reseat
  -- code reads parent_of expecting it to match `flat`.
  local parent_of = {}
  for _, s in ipairs(flat) do
    parent_of[s.id] = s.parent_id
  end
  handle.parent_of = parent_of
  -- Repaint the source buffer with the new full text. This is the
  -- one buffer write per refresh that the painting flag has to
  -- bypass; without it, the source-buffer's read-only intercept
  -- would reject our own update.
  paint_buffer(handle, handle.source, sr.text)
  -- Repaint the visible projection using the preserved fold_state.
  repaint_visible(handle)
end

-- ---------------------------------------------------------------------------
-- Intercept body for the visible buffer
-- ---------------------------------------------------------------------------
--
-- M8.5 makes the section view read-only. The visible buffer's
-- rope is a projection that the package owns; user edits would
-- desync the projection from the source + fold state. Reject
-- every kind of user edit; let our paint passes through via the
-- painting flag.

local function make_readonly_intercept(handle, name)
  return function(_op)
    if handle.painting then return nil end
    error("pmacs-magit: this buffer is a " .. name .. "; direct edits " ..
          "are not supported. Use TAB to fold/unfold sections, or " ..
          "the M-x pmacs-magit.* commands.")
  end
end

-- ---------------------------------------------------------------------------
-- Commands
-- ---------------------------------------------------------------------------

local OWNED_COMMANDS = {}

local function define_owned(spec)
  pmacs.command.define(spec)
  OWNED_COMMANDS[#OWNED_COMMANDS + 1] = spec.name
end

define_owned {
  name = "pmacs-magit.toggle-fold",
  description = "Toggle fold state of the section under cursor.",
  fn = function()
    local h = active_handle()
    if not h then return end
    local line = pmacs.editor.cursor_line()
    local id = fold.section_at(h.projection, line)
    if not id then return end  -- cursor past end-of-buffer
    fold.toggle(h.fold_state, id)
    repaint_visible(h)
  end,
}

define_owned {
  name = "pmacs-magit.fold-all",
  description = "Collapse every section in the active magit buffer.",
  fn = function()
    local h = active_handle()
    if not h then return end
    for _, s in ipairs(h.flat) do
      if #s.child_ids > 0 or #s.body_lines > 0 then
        h.fold_state[s.id] = "collapsed"
      end
    end
    repaint_visible(h)
  end,
}

define_owned {
  name = "pmacs-magit.unfold-all",
  description = "Expand every section in the active magit buffer.",
  fn = function()
    local h = active_handle()
    if not h then return end
    h.fold_state = {}
    repaint_visible(h)
  end,
}

-- ---------------------------------------------------------------------------
-- Gesture commands (T M8.7)
-- ---------------------------------------------------------------------------
--
-- Each gesture funnels through the gestures module, which knows how
-- to resolve "section / item under cursor" and runs the
-- corresponding `git` command via status.run_git. Multi-step
-- gestures (commit, push, branch-create, branch-switch) compose
-- with `pmacs.minibuffer.read` for user input; the on_accept
-- callback then schedules a fresh pmacs.async coroutine for the
-- git work.
--
-- Single-step gestures (stage, unstage) need a pmacs.async wrapper
-- because run_git yields via pmacs.workers.sleep:await(); the
-- command body is invoked from key dispatch, which is synchronous.

local function run_async(fn)
  pmacs.async(fn)
end

define_owned {
  name = "pmacs-magit.stage",
  description = "Stage the working-tree-change item under cursor.",
  fn = function()
    local h = active_handle()
    if not h or not h.repo_root then return end
    run_async(function() gestures.stage(h, update_spec) end)
  end,
}

define_owned {
  name = "pmacs-magit.unstage",
  description = "Unstage the staged item under cursor.",
  fn = function()
    local h = active_handle()
    if not h or not h.repo_root then return end
    run_async(function() gestures.unstage(h, update_spec) end)
  end,
}

define_owned {
  name = "pmacs-magit.commit",
  description = "Open a commit-message buffer for `git commit`.",
  fn = function()
    local h = active_handle()
    if not h or not h.repo_root then return end
    gestures.commit(h, update_spec)
  end,
}

define_owned {
  name = "pmacs-magit.commit-submit",
  description = "Finish the active commit-message buffer and run git commit.",
  fn = function()
    local h = commit_session_handle()
    if not h then return end
    gestures.commit_submit(h, update_spec)
  end,
}

define_owned {
  name = "pmacs-magit.commit-cancel",
  description = "Abandon the active commit-message buffer without committing.",
  fn = function()
    local h = commit_session_handle()
    if not h then return end
    gestures.commit_cancel(h)
  end,
}

define_owned {
  name = "pmacs-magit.push",
  description = "Prompt for a remote and run git push.",
  fn = function()
    local h = active_handle()
    if not h or not h.repo_root then return end
    gestures.push(h, update_spec)
  end,
}

define_owned {
  name = "pmacs-magit.branch-create",
  description = "Prompt for a branch name and run git checkout -b.",
  fn = function()
    local h = active_handle()
    if not h or not h.repo_root then return end
    gestures.branch_create(h, update_spec)
  end,
}

define_owned {
  name = "pmacs-magit.branch-switch",
  description = "Prompt with branch candidates and run git checkout.",
  fn = function()
    local h = active_handle()
    if not h or not h.repo_root then return end
    gestures.branch_switch(h, update_spec)
  end,
}

define_owned {
  name = "pmacs-magit.refresh-status",
  description = "Re-fetch git status and refresh the active magit buffer.",
  fn = function()
    local h = active_handle()
    if not h or not h.repo_root then return end
    -- Schedule an immediate refresh, separate from the polling loop's
    -- next tick. The polling loop uses h.refresh_pending to skip
    -- overlapping work; we set it here too so a manual refresh while
    -- a poll-driven one is in flight is a no-op rather than a queue.
    if h.refresh_pending then return end
    h.refresh_pending = true
    pmacs.async(function()
      local ok, err = pcall(function()
        local result = status.fetch_sections(h.repo_root)
        if h.alive then
          update_spec(h, result.spec)
          h.branches_snapshot =
            (result.parsed.branches and result.parsed.branches.all) or {}
        end
      end)
      h.refresh_pending = false
      if not ok and pmacs.editor and pmacs.editor.set_status then
        pmacs.editor.set_status(
          "pmacs-magit.refresh-status failed: " .. tostring(err)
        )
      end
    end)
  end,
}

-- ---------------------------------------------------------------------------
-- Public: open()
-- ---------------------------------------------------------------------------

-- Open a section view for `spec`. Returns the handle (mostly for
-- testing; user code typically just calls open and lets the
-- window switch handle the rest).
function M.open(spec)
  local flat = fold.parse(spec)
  if #flat == 0 then
    error("pmacs-magit.open: section spec must contain at least one section")
  end
  local source_render = fold.render_source(flat)

  -- Build the parent-of lookup from the flat parse. Used by the
  -- cursor-reseat path: when the section under cursor gets
  -- collapsed away by a parent fold, we walk up until we find an
  -- ancestor still in the visible projection.
  local parent_of = {}
  for _, s in ipairs(flat) do
    parent_of[s.id] = s.parent_id
  end

  local source = pmacs.buffer.create(" *magit-source:" .. flat[1].id .. "*")
  local visible = pmacs.buffer.create("*magit:" .. flat[1].id .. "*")

  local handle = {
    flat = flat,
    fold_state = {},
    source = source,
    visible = visible,
    -- Source rope line index, stable across fold operations.
    source_line_index = source_render.line_index,
    parent_of = parent_of,
    -- projection populated by repaint_visible below.
    projection = nil,
    painting = false,
  }
  handles[#handles + 1] = handle

  handle.source_intercept = pmacs.buffer.add_intercept(
    source, make_readonly_intercept(handle, "magit source view")
  )
  paint_buffer(handle, source, source_render.text)

  handle.intercept = pmacs.buffer.add_intercept(
    visible, make_readonly_intercept(handle, "section view")
  )
  pmacs.window.switch_buffer(visible)
  repaint_visible(handle)
  pmacs.keymap.bind {
    scope = "buffer", buffer = visible, sequence = "Tab",
    command = "pmacs-magit.toggle-fold",
  }

  return handle
end

-- ---------------------------------------------------------------------------
-- Public: open_status() --- magit-class entry for git status (T M8.6)
-- ---------------------------------------------------------------------------
--
-- Open a magit-status buffer for a Git repo. Renders the canonical
-- 5-section view (working tree, staged, recent commits, branches,
-- stashes) by invoking `git` via `pmacs.process.spawn`, parsing
-- output, and feeding the result through the M8.5 fold module.
--
-- The buffer auto-refreshes every 250 ms (well under the M8.6
-- acceptance bullet's 500 ms latency budget). The polling loop
-- terminates when the visible buffer is removed (`handle.alive`
-- flipped by cleanup_handle) or when the package is unloaded.
--
-- Refresh failures (transient git errors, missing repo, etc.) are
-- captured via pcall and surfaced through pmacs.editor.set_status
-- when available; the polling loop continues so a recoverable
-- error doesn't kill the watcher.
--
-- Returns the handle. `pmacs-magit.refresh-status` is bound to "g"
-- on the buffer so users can force-refresh manually (the canonical
-- magit gesture).
--
-- Caller invariants:
--   * `repo_root` must be an absolute path to a git repository's
--     working tree (or a subdirectory thereof; git resolves the
--     repo root itself). Validation happens at first refresh ---
--     a non-repo path produces a refresh failure with the git
--     stderr surfaced; we don't pre-validate to avoid duplicating
--     git's own logic.
--   * Must be called inside a `pmacs.async` body, because the
--     initial fetch runs synchronously to ensure the buffer
--     populates before open_status returns.

function M.open_status(repo_root)
  if type(repo_root) ~= "string" or repo_root == "" then
    error("pmacs-magit.open_status: repo_root must be a non-empty string")
  end

  -- Initial spec: a single "loading" section so M.open's "at least
  -- one section" check passes and the user sees something
  -- immediately. Replaced by the first refresh.
  local handle = M.open {
    {
      id = "_magit_status_loading",
      title = "(loading magit status...)",
      body = nil,
    },
  }
  handle.repo_root = repo_root
  handle.refresh_pending = false
  handle.alive = true

  pmacs.keymap.bind {
    scope = "buffer", buffer = handle.visible, sequence = "g",
    command = "pmacs-magit.refresh-status",
  }
  -- M8.7 gestures: stage / unstage / commit / push / branch.
  -- "b" is a prefix: "b c" creates, "b b" switches.
  pmacs.keymap.bind {
    scope = "buffer", buffer = handle.visible, sequence = "s",
    command = "pmacs-magit.stage",
  }
  pmacs.keymap.bind {
    scope = "buffer", buffer = handle.visible, sequence = "u",
    command = "pmacs-magit.unstage",
  }
  pmacs.keymap.bind {
    scope = "buffer", buffer = handle.visible, sequence = "c",
    command = "pmacs-magit.commit",
  }
  pmacs.keymap.bind {
    scope = "buffer", buffer = handle.visible, sequence = "P",
    command = "pmacs-magit.push",
  }
  pmacs.keymap.bind {
    scope = "buffer", buffer = handle.visible, sequence = "b c",
    command = "pmacs-magit.branch-create",
  }
  pmacs.keymap.bind {
    scope = "buffer", buffer = handle.visible, sequence = "b b",
    command = "pmacs-magit.branch-switch",
  }

  -- Initial fetch: synchronous via the same pcall pattern as the
  -- polling loop. We're already inside pmacs.async (callers are
  -- required to be).
  do
    handle.refresh_pending = true
    local ok, err = pcall(function()
      local result = status.fetch_sections(repo_root)
      update_spec(handle, result.spec)
      handle.branches_snapshot =
        (result.parsed.branches and result.parsed.branches.all) or {}
    end)
    handle.refresh_pending = false
    if not ok and pmacs.editor and pmacs.editor.set_status then
      pmacs.editor.set_status(
        "pmacs-magit: initial status fetch failed: " .. tostring(err)
      )
    end
  end

  -- Polling loop. One coroutine; refresh runs inline so the next
  -- sleep starts only after the previous refresh finishes ---
  -- there's no overlap, no queue, no need to debounce.
  pmacs.async(function()
    while handle.alive do
      pmacs.workers.sleep(250):await()
      if not handle.alive then break end
      -- Skip if a manual refresh is in flight (the refresh-status
      -- command sets refresh_pending). Keeps things serialized.
      if not handle.refresh_pending then
        handle.refresh_pending = true
        local ok, err = pcall(function()
          local result = status.fetch_sections(repo_root)
          if handle.alive then
            update_spec(handle, result.spec)
            handle.branches_snapshot =
              (result.parsed.branches and result.parsed.branches.all) or {}
          end
        end)
        handle.refresh_pending = false
        if not ok and pmacs.editor and pmacs.editor.set_status then
          pmacs.editor.set_status(
            "pmacs-magit: refresh failed: " .. tostring(err)
          )
        end
      end
    end
  end)

  return handle
end

-- ---------------------------------------------------------------------------
-- Cleanup on unload
-- ---------------------------------------------------------------------------

pmacs.packages.on_unload(function()
  for _, h in ipairs(handles) do
    cleanup_handle(h)
  end
  handles = {}
  for _, name in ipairs(OWNED_COMMANDS) do
    pmacs.command.unregister(name)
  end
  OWNED_COMMANDS = {}
end)

-- ---------------------------------------------------------------------------
-- Test seam
-- ---------------------------------------------------------------------------
--
-- The seam name is loud-prefixed per the M8.4 audit finding 7
-- discussion: the v0.1 audit lint can't enforce field-access
-- privacy, so the convention is to make accidental external use
-- *obvious* at the call site. External packages reaching into
-- `__pmacs_magit_test_seam_DO_NOT_USE` are unambiguously off the
-- supported path.

M.__pmacs_magit_test_seam_DO_NOT_USE = {
  active_handle = active_handle,
  fold = fold,
  status = status,
  gestures = gestures,
  repaint_visible = repaint_visible,
  paint_buffer = paint_buffer,
  update_spec = update_spec,
}

return M
