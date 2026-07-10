-- builtin/keymaps/default.lua --- M2.5 default global keymap.
--
-- Maps chord sequences onto command names defined in
-- builtin/commands/default.lua. Replaces the M1 hardcoded dispatch.
-- User config (M2.10) may unbind and rebind any of these.

local function bind(sequence, command)
  pmacs.keymap.bind { scope = "global", sequence = sequence, command = command }
end

-- Cursor motion --------------------------------------------------------------

bind("C-a",     "cursor.line-start")
bind("C-e",     "cursor.line-end")
bind("C-f",     "cursor.right")
bind("C-b",     "cursor.left")
bind("C-n",     "cursor.down")
bind("C-p",     "cursor.up")
bind("<left>",  "cursor.left")
bind("<right>", "cursor.right")
bind("<up>",    "cursor.up")
bind("<down>",  "cursor.down")
bind("<home>",  "cursor.line-start")
bind("<end>",   "cursor.line-end")

-- Word jumps (Doom-style C-left / C-right; classic Emacs M-b / M-f) ---------

bind("C-<left>",  "cursor.word-left")
bind("C-<right>", "cursor.word-right")
bind("M-b",       "cursor.word-left")
bind("M-f",       "cursor.word-right")

-- Paragraph jumps (Doom/Emacs C-up / C-down; classic M-{ / M-}) -------------

bind("C-<up>",   "cursor.paragraph-up")
bind("C-<down>", "cursor.paragraph-down")
bind("M-{",      "cursor.paragraph-up")
bind("M-}",      "cursor.paragraph-down")

-- Page motion (Page Up / Page Down; classic M-v / C-v) ---------------------

bind("<pageup>",   "cursor.page-up")
bind("<pagedown>", "cursor.page-down")
bind("M-v",        "cursor.page-up")
bind("C-v",        "cursor.page-down")

-- Editing --------------------------------------------------------------------

bind("BS",  "buffer.delete-backward")
bind("DEL", "buffer.delete-forward")
bind("C-d", "buffer.delete-forward")
bind("RET", "edit.newline-and-indent")
bind("TAB", "buffer.tab")

-- Incremental search ---------------------------------------------------------
--
-- C-s / C-r start a live isearch (forward / backward). Both keys are
-- free in the default map (save is C-x C-s, redo is C-x r), so this
-- adds isearch without colliding with the CUA / Emacs editing keys.
-- Once a search is running, C-s / C-r step to the next / previous
-- match; that interception happens in Rust, so it needs no binding.
-- C-M-s / C-M-r start a regex search (Emacs isearch-forward-regexp);
-- M-r toggles literal <-> regex mid-search (intercepted in Rust).
bind("C-s", "search.forward")
bind("C-r", "search.backward")
bind("C-M-s", "search.forward-regex")
bind("C-M-r", "search.backward-regex")

-- Query-replace (Arc 2): M-% literal, C-M-% regexp (Emacs bindings).
bind("M-%", "query-replace")
bind("C-M-%", "query-replace-regexp")

-- CUA-style word-level deletion (the same shortcuts users expect from
-- IDEs, browsers, terminals on Linux/Windows). C-BS deletes back to
-- the start of the previous word; C-DEL deletes forward through the
-- next word. Emacs's classic M-BS and M-d remain bound below.
--
-- Why we also bind C-h: most terminals (anything not implementing the
-- kitty keyboard protocol) cannot disambiguate Ctrl+Backspace from
-- Ctrl+H — both legacy paths produce byte 0x08, which crossterm
-- surfaces as `Char('h') + CONTROL`. Binding C-h to the same command
-- makes the shortcut work on legacy terminals too. C-h was free
-- (pmacs does not use it as a help prefix); users wanting Emacs's
-- help-prefix can override.
bind("C-BS",  "buffer.delete-word-backward")
bind("C-h",   "buffer.delete-word-backward")
bind("C-DEL", "buffer.delete-word-forward")
bind("M-BS",  "buffer.delete-word-backward")
bind("M-d",   "buffer.delete-word-forward")

-- CUA-style Shift+motion selection. Each Shift+arrow extends a
-- selection from the cursor (anchoring at the current position if no
-- region is yet active). Ctrl+Shift+Left/Right extend by whole words;
-- Ctrl+Shift+Up/Down extend by paragraphs; Shift+Home/End extend to
-- line edges. Plain motion (without Shift)
-- is unchanged --- it preserves any existing selection rather than
-- dropping it (Emacs-flavored default; users who want strict-CUA
-- "drop-on-plain-motion" can rebind their motion commands).
bind("S-<left>",   "cursor.select-left")
bind("S-<right>",  "cursor.select-right")
bind("S-<up>",     "cursor.select-up")
bind("S-<down>",   "cursor.select-down")
bind("S-<home>",   "cursor.select-line-start")
bind("S-<end>",    "cursor.select-line-end")
bind("C-S-<left>",  "cursor.select-word-left")
bind("C-S-<right>", "cursor.select-word-right")
bind("C-S-<up>",    "cursor.select-paragraph-up")
bind("C-S-<down>",  "cursor.select-paragraph-down")

-- Clipboard (Q#CM6). The Emacs kill/yank set --- all of these were free
-- in the default map (C-a / C-v are taken for line-start / page-down, so
-- the CUA trio would have clobbered motion). C-w cuts, M-w copies, C-y
-- pastes; C-x h selects the whole buffer (Emacs mark-whole-buffer).
bind("M-w",     "edit.copy")
bind("C-w",     "edit.cut")
bind("C-y",     "edit.paste")
bind("C-x h",   "edit.select-all")

-- Undo / redo ----------------------------------------------------------------
--
-- Multiple undo bindings exist because terminals translate Ctrl+/
-- into different byte sequences. Kitty's keyboard protocol
-- (negotiated by the frontend) routes most cleanly through C-/, but
-- the alternates remain bound so legacy or remote terminals still
-- work without reconfiguration.
bind("C-/", "buffer.undo")
bind("C-_", "buffer.undo")
bind("C-4", "buffer.undo")
bind("C-?", "buffer.redo")
bind("C-S-_", "buffer.redo")

-- Multi-key chords -----------------------------------------------------------

bind("C-x C-s", "buffer.save")
bind("C-x C-c", "editor.quit")
bind("C-x u",   "buffer.undo")
bind("C-x r",   "buffer.redo")

-- Command palette ------------------------------------------------------------

bind("M-x", "editor.execute-command")

-- Window splits and buffer switching (M2.8) ---------------------------------

bind("C-x 2",   "window.split-horizontal")
bind("C-x 3",   "window.split-vertical")
bind("C-x o",   "window.focus-next")
bind("C-x O",   "window.focus-prev")
bind("C-x 0",   "window.close")
bind("C-x 1",   "window.close-others")
bind("C-x b",       "editor.switch-buffer")
bind("C-x C-b",     "editor.list-buffers")
bind("C-x <right>", "editor.next-buffer")
bind("C-x <left>",  "editor.previous-buffer")

-- Cancellation ---------------------------------------------------------------
--
-- C-g resets the dispatcher and clears the status line. When pressed
-- inside an unfinished prefix (e.g. C-x C-g), the dispatcher reports
-- the sequence as unbound and resets, which has the same effect.
bind("C-g", "editor.cancel")
