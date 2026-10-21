-- desktop.lua --- session desktop-save wiring (Arc 3 phase 2).
--
-- The Rust `pmacs.session.{save_desktop,restore_desktop,arm_restore,
-- is_daemon}` primitives do the load-bearing work (layout serde +
-- structural rebuild). This thin layer adds the opt-in `desktop_mode`
-- switch and the manual commands.
--
-- Opt-in: nothing happens unless init.lua calls
-- `pmacs.session.desktop_mode(true)`. That switch is the in-process
-- editor's (Q#DS9): a daemon's own view is never saved or restored, and
-- since E10 the daemon keeps each attached window's desktop by its kind
-- on its own (written at detach and shutdown), so under a daemon there
-- is nothing to enable. The two commands act on the window they run in
-- there.
--
-- Framing: docs/archive/framings/desktop-save-framing.md.

local enabled = false

-- Enable (or disable) desktop-save. When enabled in local mode:
--   * arm restore-on-startup (the RunLocal trigger fires it), and
--   * save the session on quit (editor.before-quit).
function pmacs.session.desktop_mode(on)
  on = (on ~= false)
  if on and pmacs.session.is_daemon() then
    -- The daemon keeps its windows' desktops itself (E10); nothing to
    -- enable, and keep quiet rather than half-enable.
    return false
  end
  enabled = on
  -- Arm (or, when disabling, unarm) restore-on-startup, so an
  -- enable-then-disable in init.lua does not still restore.
  pmacs.session.arm_restore(on)
  return enabled
end

-- Save on quit. before-quit is short-circuit; returning nil never
-- vetoes, and a save failure must not block quitting (Q#DS8).
pmacs.hook.add("editor.before-quit", function()
  -- A daemon writes every attached window's desktop at its shutdown
  -- (E10); `enabled` can be true there only because init.lua runs
  -- before the daemon marks itself.
  if enabled and not pmacs.session.is_daemon() then
    pcall(pmacs.session.save_desktop)
  end
end)

pmacs.command.define {
  name = "desktop-save",
  description = "Save the current session (buffers + layout) to disk.",
  fn = function()
    -- Under a daemon, this window's desktop, under its kind (E10).
    local ok, wrote_or_err = pcall(pmacs.session.save_desktop)
    if not ok then
      pmacs.editor.set_status("desktop-save: " .. tostring(wrote_or_err))
    elseif wrote_or_err then
      pmacs.editor.set_status("desktop saved")
    else
      pmacs.editor.set_status("desktop-save: nothing to save")
    end
  end,
}

pmacs.command.define {
  name = "desktop-restore",
  description = "Restore the saved session (buffers + layout) from disk.",
  fn = function()
    -- Under a daemon, this window's kind's desktop, into this window (E10).
    local ok, err = pcall(pmacs.session.restore_desktop)
    if not ok then
      pmacs.editor.set_status("desktop-restore: " .. tostring(err))
    end
  end,
}
