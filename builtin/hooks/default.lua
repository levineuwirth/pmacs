-- builtin/hooks/default.lua --- M2.6 + M4.12 lifecycle hooks.
--
-- Defines the named hooks the editor itself fires:
--   * buffer.before-save  --- short-circuit. A callback may veto the
--                             save by returning false (e.g. a linter
--                             that refuses to save while errors exist).
--   * buffer.after-load   --- all-must-succeed. Every listener runs
--                             when a file loads; failures are logged
--                             to *errors* but don't block the others.
--   * buffer.after-edit   --- all-must-succeed (M4.12). Fired after a
--                             key dispatch that mutated the active
--                             buffer; LSP did_change wiring debounces
--                             this. Listeners receive no arguments.
--   * buffer.after-save   --- all-must-succeed (M4.12). Fired after a
--                             successful save; LSP did_save and
--                             format-on-save subscribe here.
--   * editor.before-quit  --- short-circuit. A callback may veto quit
--                             (e.g. "buffer modified --- save first?").
--
-- These are *defined* here so user config can attach callbacks via
-- pmacs.hook.add. Run sites are in Rust (after-load, after-edit) and in
-- builtin/commands/default.lua (before-save, after-save, before-quit).

local define = pmacs.hook.define

define {
  name = "buffer.before-save",
  description = "Fired before pmacs.editor.save writes the active buffer to disk. " ..
                "Return false to veto the save.",
  kind = "short-circuit",
}

define {
  name = "buffer.after-load",
  description = "Fired after a file is read into a buffer. Listeners " ..
                "run independently; one failure does not block the rest.",
  kind = "all-must-succeed",
}

define {
  name = "buffer.after-edit",
  description = "Fired after a key dispatch that modifies the active buffer. " ..
                "LSP did_change subscribers debounce this hook.",
  kind = "all-must-succeed",
}

define {
  name = "buffer.after-save",
  description = "Fired after a successful save. LSP did_save and " ..
                "format-on-save subscribers fire here.",
  kind = "all-must-succeed",
}

define {
  name = "editor.before-quit",
  description = "Fired before the editor exits. Return false to veto.",
  kind = "short-circuit",
}

define {
  name = "process.after-tick",
  description = "Fired once per editor frame, immediately after the process " ..
                "supervisor's tick drains pending I/O and exit events. " ..
                "Subscribers typically call pmacs.process.events_take(id) " ..
                "to drain events for processes they care about. The hook " ..
                "fires unconditionally each frame; an empty supervisor still " ..
                "fires the hook (subscribers carry their own registries).",
  kind = "all-must-succeed",
}
