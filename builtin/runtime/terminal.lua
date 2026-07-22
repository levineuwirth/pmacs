-- terminal.lua --- Friendly Vterm Stage 2 command and modeline surface.

local terminal = assert(pmacs.terminal, "pmacs.terminal raw bindings are required")
local raw_open = assert(terminal._open, "pmacs.terminal._open is required")

local function bind_terminal_keys(buffer)
  local function bind(sequence, command)
    pmacs.keymap.bind {
      scope = "buffer",
      buffer = buffer,
      sequence = sequence,
      command = command,
    }
  end
  bind("M-w", "terminal.copy-selection")
  bind("M-v", "terminal.page-up")
  bind("C-v", "terminal.page-down")
  bind("M-<", "terminal.scroll-oldest")
  bind("M->", "terminal.scroll-bottom")
end

function terminal.open(spec)
  local buffer = raw_open(spec)
  bind_terminal_keys(buffer)
  return buffer
end

pmacs.command.define {
  name = "terminal",
  description = "Open a terminal running $SHELL (or /bin/sh).",
  fn = function()
    return terminal.open {
      command = os.getenv("SHELL") or "/bin/sh",
    }
  end,
}

pmacs.command.define {
  name = "terminal.copy-selection",
  description = "Copy the active terminal selection.",
  fn = function() return terminal.copy_selection() end,
}

pmacs.command.define {
  name = "terminal.page-up",
  description = "Scroll the active terminal viewport up one page.",
  fn = function() return terminal._scroll_page(1) end,
}

pmacs.command.define {
  name = "terminal.page-down",
  description = "Scroll the active terminal viewport down one page.",
  fn = function() return terminal._scroll_page(-1) end,
}

pmacs.command.define {
  name = "terminal.scroll-oldest",
  description = "Scroll the active terminal viewport to the oldest retained row.",
  fn = function() return terminal.scroll(math.maxinteger) end,
}

pmacs.command.define {
  name = "terminal.scroll-bottom",
  description = "Return the active terminal viewport to the live tail.",
  fn = function() return terminal.scroll_to_bottom() end,
}

pmacs.statusline.register {
  name = "terminal",
  side = "right",
  priority = 10,
  face = "ui.modeline.terminal",
  fn = function(ctx)
    if not terminal.is_terminal(ctx.buffer) then return nil end
    local state = terminal.state(ctx.buffer)
    local view = terminal.view_state(ctx)
    if not view then return nil end

    local process = state.process
    local text
    if process.kind == "running" then
      text = "TERM"
    elseif process.kind == "exited" then
      text = "TERM:" .. tostring(process.code)
    elseif process.kind == "signaled" then
      text = "TERM:" .. process.signal
    else
      text = "TERM:ERR"
    end
    if view.scroll_offset > 0 then
      text = text .. " ↑" .. tostring(view.scroll_offset)
    end
    return text
  end,
}
