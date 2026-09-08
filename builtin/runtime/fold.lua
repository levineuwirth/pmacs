-- fold.lua --- interactive fold commands + default bindings (Arc 6).
--
-- Thin command wrappers over the `pmacs.fold` Rust surface. Each resolves
-- the invoking frontend's active-window buffer and point (command context,
-- not an ambient buffer), then calls the state-aware helper; the Rust side
-- refuses on a stale/absent parse tree, validates, and moves the point to
-- the head line when it folds around it.
--
-- Default bindings are the Emacs hideshow `C-c @` prefix set (Q#FD4): the
-- LSP surface already owns every `C-c <letter>`, so `C-c @` is the one
-- faithful prefix that collides with nothing. Rebind through pmacs.keymap.
--
-- Framing: docs/archive/framings/folding-framing.md.

local ed = pmacs.editor
local fold = pmacs.fold

pmacs.command.define {
  name = "fold.toggle",
  description = "Toggle the fold at point (org-TAB cycle)",
  fn = function() fold.cycle(pmacs.window.buffer(), ed.cursor()) end,
}

pmacs.command.define {
  name = "fold.close",
  description = "Close the innermost open fold at point",
  fn = function() fold.close(pmacs.window.buffer(), ed.cursor()) end,
}

pmacs.command.define {
  name = "fold.open",
  description = "Open the outermost closed fold at point",
  fn = function() fold.open(pmacs.window.buffer(), ed.cursor()) end,
}

pmacs.command.define {
  name = "fold.close-all",
  description = "Close all top-level folds in the buffer",
  fn = function() fold.close_all(pmacs.window.buffer()) end,
}

pmacs.command.define {
  name = "fold.open-all",
  description = "Open all folds in the buffer",
  fn = function() fold.open_all(pmacs.window.buffer()) end,
}

pmacs.keymap.bind { scope = "global", sequence = "C-c @ C-c", command = "fold.toggle" }
pmacs.keymap.bind { scope = "global", sequence = "C-c @ C-h", command = "fold.close" }
pmacs.keymap.bind { scope = "global", sequence = "C-c @ C-s", command = "fold.open" }
pmacs.keymap.bind { scope = "global", sequence = "C-c @ C-M-h", command = "fold.close-all" }
pmacs.keymap.bind { scope = "global", sequence = "C-c @ C-M-s", command = "fold.open-all" }
