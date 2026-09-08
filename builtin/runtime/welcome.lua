-- welcome.lua --- journey step 4: say something when the editor opens.
-- Framing: docs/archive/framings/journey-stage1b3-welcome-framing.md.
--
-- `COHERENCE.md` §18 graded onboarding "missing entirely": no welcome,
-- no tutorial, no cheat sheet reachable from inside the editor. The sole
-- discovery affordance was knowing to press `M-x`.
--
-- Split of responsibility with Rust: this file owns WHAT is said (the
-- entries and their rendering) and the `help` command; the Rust seam
-- `EditorState::finalize_local_launch` owns WHEN and WHERE — it alone
-- decides that this is a local, no-target launch whose `*scratch*` is
-- still untouched, and it clears the modified flag afterwards (there is
-- no Lua API for that, deliberately).

pmacs.welcome = pmacs.welcome or {}

--- The keys the welcome advertises, in display order.
---
--- `keys` is EXACTLY what `pmacs.keymap.lookup` accepts, which is the
--- whole point of the shape: the acceptance suite checks every entry
--- resolves, so the welcome can never advertise a binding a later stage
--- removed. Scraping the rendered prose instead would be ambiguous —
--- `C-c c` is two chords and nothing in the text marks the boundary.
---
--- Public so a user who rebinds can rebuild it from `init.lua`.
pmacs.welcome.entries = {
  { keys = "C-x C-f", label = "open a file"   },
  { keys = "C-c t",   label = "terminal"      },
  { keys = "C-c c",   label = "build"         },
  { keys = "C-x b",   label = "switch buffer" },
}

-- Two entries per line, padded so the labels align. Kept deliberately
-- small: three lines total, because the greeting a user must delete
-- before typing should not be chrome.
local function entry_columns()
  local width = 0
  for _, e in ipairs(pmacs.welcome.entries) do
    if #e.keys > width then width = #e.keys end
  end
  local lines, pending = {}, nil
  for _, e in ipairs(pmacs.welcome.entries) do
    local cell = string.format("%-" .. width .. "s  %s", e.keys, e.label)
    if pending then
      lines[#lines + 1] = "  " .. string.format("%-24s", pending) .. cell
      pending = nil
    else
      pending = cell
    end
  end
  if pending then lines[#lines + 1] = "  " .. pending end
  return lines
end

--- The welcome text, as written into an untouched `*scratch*`.
---
--- `M-x` and `M-x help` are prose rather than entries: `M-x` is the
--- palette itself and `help` is a command name, so neither is a keymap
--- lookup. The acceptance checks the command exists instead.
function pmacs.welcome.text()
  local lines = { "Welcome to pmacs.  M-x runs any command; M-x help lists the keys." }
  for _, line in ipairs(entry_columns()) do
    lines[#lines + 1] = line
  end
  return table.concat(lines, "\n") .. "\n"
end

-- ---------------------------------------------------------------------
-- M-x help
-- ---------------------------------------------------------------------
--
-- The `help` command itself lives in `runtime/help.lua`, which owns the
-- whole discovery family and loads after this file so its index can read
-- `pmacs.welcome.entries` above. This file keeps only the greeting.
