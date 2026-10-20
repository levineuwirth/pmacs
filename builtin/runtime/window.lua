-- window.lua --- side-window settings, quit, and keyboard resize.
--
-- The Lua half of the bottom-panel arc's window surface. The placement
-- policy itself is Rust (`pmacs.window.display` / `display_file` /
-- `quit` / `resize`); this module owns the two settings those paths
-- resolve, plus the interactive commands and their Emacs bindings.
--
-- Both settings are read against the window's OWN buffer (buffer-local
-- override -> global -> default), so a project or a mode hook can pin a
-- taller panel for one buffer with `pmacs.config.set_local`.
--
-- Framing: docs/archive/framings/bottom-panel-framing.md (Q#BP2, Q#BP5b, Q#BP11).

-- Outer rows (text + mode line) a freshly created panel takes when the
-- caller supplies no explicit `height`. Only consulted at CREATION: a
-- replacement preserves whatever height the user dragged the slot to.
pmacs.config.define {
  name = "window.panel-height",
  description = "Outer rows a newly created bottom panel occupies.",
  type = "integer",
  default = 12,
  min = 2,
  mutability = "live",
}

-- A preference, not a structural rule: it constrains INTERACTIVE resize
-- (drag and the commands below) and is deliberately ignored by the
-- ordinary layout pass and by frame-resize reconciliation, so raising it
-- can never invalidate a layout that already exists.
--
-- The registry floor is 1 rather than 2 on purpose: a value below the
-- STRUCTURAL floor is clamped when it is read, not rejected when it is
-- written, so a user who asks for a smaller minimum simply gets the
-- smallest one the layout can actually honor.
pmacs.config.define {
  name = "window.min-height",
  description = "Smallest outer rows interactive resize will leave a window.",
  type = "integer",
  default = 2,
  min = 1,
  mutability = "live",
}

-- E9: how long the daemon keeps the view of a frontend whose socket was
-- lost --- its window, buffer, cursor and scroll --- for the session
-- token that frontend holds, so a GUI that loses its socket and
-- reconnects lands where it was. A frontend that said it was leaving
-- (its window's close, its own quit) keeps nothing; a GUI window closed
-- with C-x C-c while lost had no socket to say so on, so the daemon saw
-- a loss and keeps its view (E9 fix round 2). The retained windows are
-- still updated by every edit (E6d.5 measured what leaked windows cost),
-- so the window is bounded; 0 retains nothing, and a frontend
-- reconnecting later than this starts on a fresh view. Read by the
-- daemon on every tick.
pmacs.config.define {
  name = "session.view-retention-secs",
  description = "Seconds the window, buffer and cursor of a frontend whose socket was lost are kept for its session token, so a reconnecting GUI resumes where it was; a frontend that detaches or quits keeps nothing, and a GUI window closed with C-x C-c while lost counts as lost. 0 keeps nothing.",
  type = "integer",
  default = 600,
  min = 0,
  mutability = "live",
}

-- E9.4: a daemon with no frontend attached for this long stops, as
-- `pmacs --daemon --stop` would stop it. Opt-in: the default never
-- stops, since a daemon the user started by hand is theirs to stop. The
-- case it exists for is the daemon `pmacs --gpu` starts for itself,
-- which otherwise outlives every window it ever served.
pmacs.config.define {
  name = "daemon.idle-exit-secs",
  description = "Seconds a daemon stays up with no frontend attached, and no window's view retained for a lost socket, before it stops on its own; 0 never stops it.",
  type = "integer",
  default = 0,
  min = 0,
  mutability = "live",
}

pmacs.command.define {
  name = "window.quit",
  description = "Quit the selected side window: restore or delete it",
  fn = function() pmacs.window.quit() end,
}

pmacs.command.define {
  name = "window.enlarge",
  description = "Make the selected window one row taller",
  fn = function() pmacs.window.resize(nil, 1) end,
}

pmacs.command.define {
  name = "window.shrink",
  description = "Make the selected window one row shorter",
  fn = function() pmacs.window.resize(nil, -1) end,
}

pmacs.keymap.bind { scope = "global", sequence = "C-x ^", command = "window.enlarge" }
pmacs.keymap.bind { scope = "global", sequence = "C-x C-^", command = "window.shrink" }
