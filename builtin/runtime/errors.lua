-- errors.lua --- the error channel's mode-line mark (E5.1).
--
-- `pmacs.error(message [, label])` is Rust-installed and is the one
-- reporting channel: it appends `[label] message` to `*errors*`, pushes
-- the record onto the log the status line reads, and counts it unread.
-- This chunk owns the other half of "an unread mark in the modeline": a
-- statusline provider that shows how many reports nobody has looked at,
-- on every window's mode line, on both frontends (the grid paints custom
-- segments; a semantic frontend receives them as `StatuslineSegments`).
-- Showing `*errors*` in any window clears it (`mark_errors_read_if_shown`
-- in `src/editor.rs`, called by both painters), and so does
-- `pmacs.error_log.mark_read()`.
--
-- Loaded first among the runtime chunks: it needs only `pmacs.statusline`
-- and `pmacs.error_log`, and every later chunk may report.

pmacs.statusline.register {
  name = "errors",
  side = "right",
  -- Above the LSP segment (priority 0): a report the user has not seen
  -- outranks a server label when the mode line is too narrow for both.
  priority = 10,
  fn = function()
    local n = pmacs.error_log.unread()
    if n > 0 then return string.format("!%d", n) end
    return nil
  end,
}
