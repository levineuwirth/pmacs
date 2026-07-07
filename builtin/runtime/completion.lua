-- completion.lua --- in-buffer completion popup driver (Arc 1a).
--
-- Wires the M4.11 provider framework (`pmacs.completion.collect`) and
-- the M4.7 LSP request path to the core's popup session
-- (`pmacs.completion.popup_show/hide`, Q#C2). The dispatcher owns
-- navigation and accept (Q#C3/Q#C7); this file decides WHEN the popup
-- opens, WHAT it shows, and keeps it fresh as the user types.
--
-- Trigger policy (Q#C9): `buffer.after-edit` carries no payload, so
-- intent is reconstructed from state. A snapshot of {buffer, cursor}
-- from the previous invocation recognizes the single-char typing
-- signature (cursor advanced exactly one byte --- word and LSP
-- trigger characters are all ASCII); paste, undo, kill, and remote
-- edits (any other delta) never auto-open. `C-M-i`
-- (`completion.at-point`) covers deliberate invocation.
--
-- Framing: docs/in-buffer-completion-framing.md.

local MIN_PREFIX = 2 -- typed word length before the popup auto-opens
local MAX_ROWS = 64 -- cap on candidates published to the session

-- Snapshot of the previous after-edit invocation (Q#C9).
local last = { key = nil, cursor = nil }

-- Driver-side mirror of the session we opened: { key, anchor,
-- pending }. `popup_visible()` is the truth about the popup --- the
-- core closes it independently (validation, accept, dismiss, a modal
-- opening) --- so the mirror only remembers the anchor and detects
-- "the core closed it since we last looked", which doubles as the
-- reopen-after-accept suppressor. `pending = true` marks a session
-- whose popup hasn't opened yet (awaiting the LSP response).
local session = nil

local function word_prefix_before(buf, cursor)
  local start = cursor - 64
  if start < 0 then start = 0 end
  local ok, chunk = pcall(function() return buf:slice(start, cursor) end)
  if not ok or type(chunk) ~= "string" then return "" end
  return chunk:match("[%w_]*$") or ""
end

local function char_before(buf, cursor)
  if cursor < 1 then return nil end
  local ok, ch = pcall(function() return buf:slice(cursor - 1, cursor) end)
  if ok and type(ch) == "string" and #ch == 1 then return ch end
  return nil
end

local function close_popup()
  session = nil
  if pmacs.completion.popup_visible() then pmacs.completion.popup_hide() end
end

-- Collect through the framework, drop non-matches (collect keeps
-- negative-score rows, merely sorted last --- Q#C1), cap, and shape
-- rows for popup_show. Returns the rows plus the uncapped match count.
local function collect_rows(buf, prefix, trigger, trigger_char)
  local rec = pmacs.lsp.active_attachment() -- peek: uri/language only
  local ok_text, text = pcall(function() return buf:slice(0, buf:len()) end)
  if not ok_text or type(text) ~= "string" then return {}, 0 end
  local ctx = {
    prefix = prefix,
    line = pmacs.editor.cursor_line(),
    col = pmacs.editor.cursor_col(),
    buffer_text = text,
    language = rec and rec.language or nil,
    uri = rec and rec.uri or nil, -- Q#C8: scope URI-keyed providers
    trigger = trigger,
    trigger_char = trigger_char,
  }
  local ok, cands = pcall(pmacs.completion.collect, ctx)
  if not ok or type(cands) ~= "table" then return {}, 0 end
  local rows, total = {}, 0
  for _, c in ipairs(cands) do
    if (c.score or -1) >= 0 then
      total = total + 1
      if #rows < MAX_ROWS then
        rows[#rows + 1] = {
          label = c.label,
          kind = c.kind,
          detail = c.detail,
          insert_text = c.insert_text,
        }
      end
    end
  end
  return rows, total
end

-- Re-collect and show the session at `anchor`. On zero matches the
-- popup hides; the caller decides whether the session survives as
-- `pending` (initial trigger-char / at-point opens awaiting the LSP)
-- or dies (a refresh that narrowed to nothing). Returns true when the
-- popup is showing afterwards.
local function publish(buf, anchor, prefix, trigger, trigger_char)
  local rows, total = collect_rows(buf, prefix, trigger, trigger_char)
  if #rows == 0 then
    if pmacs.completion.popup_visible() then pmacs.completion.popup_hide() end
    return false
  end
  session = { key = tostring(buf), anchor = anchor }
  pmacs.completion.popup_show {
    buffer = buf,
    anchor = anchor,
    prefix = prefix,
    total = total,
    candidates = rows,
  }
  return true
end

-- Q#C8 "show fast, refresh on arrival": fire textDocument/completion
-- through the FLUSHING accessor (the server must see current text),
-- then re-publish when the response lands --- if the session is still
-- anchored where it was when the request left.
local function request_lsp_then_refresh()
  local rec = pmacs.lsp.attachment_for_request()
  if not rec or not session then return end
  local line = pmacs.editor.cursor_line()
  local col = pmacs.editor.cursor_col()
  local anchor_at_request = session.anchor
  local key_at_request = session.key
  pmacs.async(function()
    local ok = pcall(function()
      pmacs.lsp.request_completion(rec.server, rec.uri, line, col):await()
    end)
    if not ok or not session then return end
    if session.key ~= key_at_request or session.anchor ~= anchor_at_request then return end
    local buf = pmacs.window.buffer()
    if not buf or tostring(buf) ~= session.key then return end
    local cursor = pmacs.editor.cursor()
    local prefix = word_prefix_before(buf, cursor)
    if cursor - #prefix ~= session.anchor then return end
    if not publish(buf, session.anchor, prefix, "incomplete", nil) and session.pending then
      -- Still nothing, even with the server's answer: the pending
      -- session is dead.
      session = nil
    end
  end)
end

pmacs.hook.add("buffer.after-edit", function()
  local buf = pmacs.window.buffer()
  if not buf then
    close_popup()
    last.key, last.cursor = nil, nil
    return
  end
  local key = tostring(buf)
  local cursor = pmacs.editor.cursor()
  local prev_key, prev_cursor = last.key, last.cursor
  last.key, last.cursor = key, cursor

  local visible = pmacs.completion.popup_visible()

  if session and not session.pending and not visible then
    -- The core closed the popup since we opened it (accept, dismiss,
    -- validation, or a modal). Drop the mirror and do NOT reopen off
    -- this same edit --- this is what stops an accept's own
    -- after-edit from instantly re-raising the popup it just closed.
    session = nil
    return
  end

  if visible and session then
    -- Refresh the open session from the text. A prefix that no longer
    -- reaches back to the anchor means the word died; close (the
    -- core's post-dispatch validation independently enforces the same
    -- invariant).
    if key ~= session.key then
      close_popup()
      return
    end
    local prefix = word_prefix_before(buf, cursor)
    if cursor < session.anchor or cursor - #prefix ~= session.anchor then
      close_popup()
      return
    end
    if publish(buf, session.anchor, prefix, "incomplete", nil) then
      -- isIncomplete contract: a partial server response must be
      -- re-queried as the user keeps typing, not merely re-filtered.
      local rec = pmacs.lsp.active_attachment()
      if rec then
        local ok, incomplete = pcall(pmacs.completion.is_incomplete, rec.server, rec.uri)
        if ok and incomplete then request_lsp_then_refresh() end
      end
    else
      session = nil -- narrowed to nothing: the session is over
    end
    return
  end

  -- Popup closed: the Q#C9 auto-open policy. Same buffer, cursor
  -- advanced by exactly one byte since the previous edit.
  if key ~= prev_key or not prev_cursor or cursor - prev_cursor ~= 1 then return end
  local prefix = word_prefix_before(buf, cursor)
  if #prefix >= MIN_PREFIX then
    -- Fire the LSP request even when the synchronous providers came
    -- up empty: for an LSP-only word (no dabbrev/snippet/index hit,
    -- cold store) the popup materializes when the response lands ---
    -- the same pending-session shape as the trigger-char path.
    if not publish(buf, cursor - #prefix, prefix, "invoked", nil) then
      session = { key = key, anchor = cursor - #prefix, pending = true }
    end
    request_lsp_then_refresh()
    return
  end
  if #prefix == 0 then
    -- Maybe a server trigger character (`.`, `:`, ...): a pending
    -- session anchored at the cursor, opening when candidates arrive.
    local ch = char_before(buf, cursor)
    local rec = pmacs.lsp.active_attachment()
    if not (ch and rec) then return end
    local ok, fires = pcall(pmacs.completion.should_fire, rec.server, ch)
    if not (ok and fires) then return end
    if not publish(buf, cursor, "", "char", ch) then
      session = { key = key, anchor = cursor, pending = true }
    end
    request_lsp_then_refresh()
  end
end)

local function completion_at_point()
  local buf = pmacs.window.buffer()
  if not buf then return end
  local cursor = pmacs.editor.cursor()
  local prefix = word_prefix_before(buf, cursor)
  local anchor = cursor - #prefix
  if not publish(buf, anchor, prefix, "invoked", nil) then
    session = { key = tostring(buf), anchor = anchor, pending = true }
  end
  request_lsp_then_refresh()
end

pmacs.command.define {
  name = "completion.at-point",
  description = "Open the in-buffer completion popup at the cursor.",
  fn = completion_at_point,
}

pmacs.keymap.bind { scope = "global", sequence = "C-M-i", command = "completion.at-point" }
