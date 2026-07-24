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
-- Framing: docs/bottom-panel-framing.md (Q#BP2, Q#BP5b, Q#BP11).

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
