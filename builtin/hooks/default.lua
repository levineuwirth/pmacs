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
--   * resource.renamed    --- all-must-succeed (dired Stage 2a). Fired
--                             after a successful rename, with (old, new)
--                             canonical absolute paths.
--   * resource.deleted    --- all-must-succeed (dired Stage 2a). Fired
--                             after a successful delete, with the
--                             canonical absolute path.
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
  name = "buffer.after-switch",
  description = "Fired after the active window switches to a different, " ..
                "already-open buffer (C-x b, panel visits, find_or_open of " ..
                "an open file). Switching clears the window's overlays; " ..
                "syntax/LSP subscribers re-attach theirs here. Fresh loads " ..
                "fire buffer.after-load instead.",
  kind = "all-must-succeed",
}

define {
  name = "buffer.after-save",
  description = "Fired after a successful save. LSP did_save and " ..
                "format-on-save subscribers fire here.",
  kind = "all-must-succeed",
}

define {
  name = "path.open-directory",
  description = "Fired when a directory path is opened (Journey Stage 1a). " ..
                "Receives the canonical absolute path and an opaque " ..
                "destination. Return false to CLAIM the directory and stop " ..
                "the fan-out; return nothing to decline. No builtin " ..
                "subscribes -- because hook callbacks only ever append, a " ..
                "subscribing builtin would always claim before any user " ..
                "listener could run, so this hook is the user's chain and " ..
                "pmacs.path.directory_handler is the default surface it " ..
                "falls back to. A callback that RAISES stops the chain and " ..
                "suppresses that fallback.",
  kind = "short-circuit",
}

define {
  name = "resource.renamed",
  description = "Fired once per SUCCESSFUL filesystem rename, with the old " ..
                "and new paths as canonical absolute strings. The core " ..
                "reconciles what it can reach -- buffer paths and names, the " ..
                "URI-keyed LSP stores, attached diagnostic overlays -- but a " ..
                "package that keys its own state by path or URI is invisible " ..
                "to that, so this hook is the mechanism that scales. It " ..
                "carries PATHS rather than a rebind list precisely because " ..
                "dired's listing buffers are pathless: a path-keyed consumer " ..
                "must be able to reconcile from (old, new) alone. Does not " ..
                "fire for a rename that failed or was cancelled.",
  kind = "all-must-succeed",
}

define {
  name = "resource.deleted",
  description = "Fired once per SUCCESSFUL filesystem delete, with the " ..
                "canonical absolute path. Buffers on the path and beneath it " ..
                "have already been reconciled: unmodified ones killed " ..
                "through both removal phases, modified ones kept alive. " ..
                "Subscribers drop their own path-keyed state. Does not fire " ..
                "for a delete that failed or was cancelled.",
  kind = "all-must-succeed",
}

define {
  name = "editor.before-quit",
  description = "Fired before the editor exits. Return false to veto.",
  kind = "short-circuit",
}

define {
  name = "frontend.detached",
  description = "Fired when an attached frontend's session ends. " ..
                "Receives the raw frontend id (integer). Modules keying " ..
                "state by pmacs.frontend.id() (kill-ring sessions) drop " ..
                "that id's entries here (Q#KR11).",
  kind = "all-must-succeed",
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
