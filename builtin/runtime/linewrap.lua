-- Long lines (QoL Stage 3, framing docs/archive/framings/long-lines-framing.md).
--
-- Declares `ui.line-wrap`. Everything that honors it is Rust: the grid
-- renderer walks it through `Viewport`, the coordinate mapping takes it
-- through `LayoutCtx`, and semantic frontends are told over
-- `InstanceMessage::LineWrapFacts` at protocol v22 because they lay out
-- locally and would otherwise never hear it.
--
-- Buffer-local (Q#LL2). The registry already supports a per-buffer
-- layer, and this is a property of the content: prose wants wrapping,
-- a log file usually does not. The *anchor* the viewport scrolls to
-- stays per-window, because two panes on one buffer scroll
-- independently.
--
-- `ui.`, not `editing.`: `editing.*` is buffer-editing behavior
-- (auto-pair, trim-on-save, line endings) and this changes only how
-- text is shown. The two existing `ui.*` settings carry a `gpu-`
-- prefix to mark frontend-specific ones, so the ABSENCE of a prefix
-- here is what says "both frontends".

pmacs.config.define {
  name = "ui.line-wrap",
  -- The description names what `truncate` COSTS, because the toggle's
  -- status message is not enough: a user who sets this in `init.lua`
  -- never invokes the toggle and so never sees it. #221 shipped that
  -- gap (framing §6).
  --
  -- In the TUI the cost is now only the GUI's, because Stage 4 gave the
  -- grid renderer horizontal scrolling. Stage 5 closes the rest, and
  -- this sentence shrinks back when it lands.
  description = "How a line wider than the window is shown: wrap onto following rows, or truncate at the edge. Truncated text is reachable by moving the cursor past the edge in the terminal UI; in the GUI it is not yet reachable at all.",
  -- A closed set, so an unknown value is impossible rather than
  -- handled. Adding "word" later is a clean additive change --- which
  -- is the plan, since character wrap is what both frontends can do
  -- identically today (Q#LL5) and word wrap is a deliberate future
  -- choice rather than an inherited library default.
  type = "enum",
  choices = { "wrap", "truncate" },
  -- `wrap` is the only value that leaves every character reachable
  -- with this stage's machinery. It is also what the GPU already did,
  -- so the default is not a behavior change there --- but it IS one in
  -- the TUI, which truncated. No default can preserve both, because
  -- the two frontends disagreed before this setting existed; that is
  -- the defect, not a side effect of fixing it.
  default = "wrap",
  mutability = "live",
}

-- `truncate` leaves text past the right edge UNREACHABLE until Stage 4
-- adds horizontal scrolling. That is stated in the description above
-- rather than left for a user to discover, and it is why `truncate` is
-- not the default despite being the TUI's historical behavior.

-- Buffer-local on BOTH sides, and the pairing matters.
--
-- `pmacs.config.get(name)` reads the global chain and
-- `pmacs.config.set(name, ...)` writes the global layer, so a toggle
-- built from those two would be wrong in the case the setting exists
-- for: a buffer pinned to `truncate` would report the GLOBAL value,
-- flip the GLOBAL value, and leave that buffer exactly as it was ---
-- while silently changing every buffer that had not been pinned.
--
-- So: read with `get(name, buf)`, write with `set_local(buf, ...)`,
-- and resolve the buffer ONCE for both. Resolving twice would be a
-- narrower version of the same bug, since the active buffer can change
-- between two calls.
pmacs.command.define {
  name = "ui.toggle-line-wrap",
  description = "Toggle line wrapping for the current buffer",
  fn = function()
    local buf = pmacs.window.buffer()
    local current = pmacs.config.get("ui.line-wrap", buf)
    local next_mode = current == "wrap" and "truncate" or "wrap"
    pmacs.config.set_local(buf, "ui.line-wrap", next_mode)
    if next_mode == "truncate" then
      pmacs.editor.set_status("line wrap off — move the cursor past the edge to scroll (GUI: not yet)")
    else
      pmacs.editor.set_status("line wrap on")
    end
  end,
}
