-- GUI zoom (QoL Stage 2, framing docs/gui-zoom-framing.md).
--
-- Drives the font preference that already exists: `pmacs.gpu.set_font`
-- writes it, `semantic_render` relays it as `InstanceMessage::FontFacts`
-- at protocol v17, and the GPU frontend owns every pixel consequence.
-- Nothing here knows a metric, an advance, or what resolves --- the
-- no-pixels invariant (src/font_pref.rs) holds through this module.
--
-- NO KEYBINDINGS, deliberately (Q#Z3). `keymap_stack::Scope` is
-- Buffer | Mode | Global and carries no frontend identity, so "bind
-- this on GPU frontends only" is not expressible; and `FrontendEvent`
-- has no command-invocation variant, so the GPU cannot ask for a
-- command by name either. A global binding would capture C-+/C-- in
-- the TUI and take away the terminal's own zoom --- the very thing the
-- user is pressing the key for. Commands are discoverable via M-x and
-- one line to bind in init.lua; capability-aware binding is its own
-- lane.

pmacs.zoom = pmacs.zoom or {}

-- Wire bounds, in logical px: `FONT_SIZE_CENTI_PX_RANGE` is 600..=7200
-- (pmacs-gpu/src/main.rs). Mirrored rather than imported because the
-- Lua range check is a UX courtesy; the frontend re-checks on arrival
-- because that side is deserialized protocol input.
local MIN_PX = 6.0
local MAX_PX = 72.0

local STATE_KEY = "gpu-zoom"

pmacs.config.define {
  name = "ui.gpu-font-size-base",
  description = "Logical-pixel size the first zoom step starts from when no font size is set (quantized to hundredths).",
  type = "number",
  default = 16.0,
  min = MIN_PX,
  max = MAX_PX,
  mutability = "live",
}

-- Lower bound is the QUANTIZER, not tidiness: `validate_font_size`
-- rounds to the nearest hundredth, so a step below 0.01 quantizes to
-- zero and "zoom in" silently does nothing forever. A negative step
-- would invert the commands --- zoom-in shrinking is not a malfunction
-- the user can diagnose, because the command still does something
-- coherent. Upper bound is the range span (72 - 6): a larger step can
-- only ever clamp.
pmacs.config.define {
  name = "ui.gpu-zoom-step",
  description = "Logical pixels added or removed per zoom step (quantized to hundredths).",
  type = "number",
  default = 1.0,
  min = 0.01,
  max = 66.0,
  mutability = "live",
}

-- Round to centi-pixel, the wire's unit. Doing this here keeps every
-- comparison and every round-trip in the quantized domain, so "n in,
-- n out" is exact addition rather than float drift.
local function quantize(px)
  return math.floor(px * 100 + 0.5) / 100
end

-- The configured step and base, QUANTIZED.
--
-- The registry cannot enforce this: `ConfigKind::Number` validates
-- finiteness and bounds and nothing else (src/config_registry.rs), and
-- `on_change` listeners are notified after the fact --- they cannot
-- veto. A wrapper function would not help either, since a direct
-- `pmacs.config.set` bypasses it (the same seam `autosave` documents).
--
-- So quantize where the value is USED. A step of 0.015 is not a
-- meaningful step in this domain: sizes live in integer hundredths of a
-- logical pixel end to end, and `validate_font_size` already
-- range-checks the original and then rounds to the nearest hundredth.
-- Rounding the step is the same operation applied one level up, not a
-- workaround for one.
--
-- It also RESTORES the round-trip contract, which a raw step breaks:
-- with 0.015 the sequence is 16.00 -> 16.02 -> 16.01. Each operation
-- rounds independently, and both intermediates land on an EXACT tie ---
-- 16.015 and 16.005 are 1601.5 and 1600.5 centi-px --- which this
-- half-up quantizer sends UP. Half-up is not symmetric under negation:
-- rounding up on the way in adds half a centi-pixel, and rounding up on
-- the way out adds another, so the two errors accumulate instead of
-- cancelling. Quantizing first makes every step exact addition in the
-- quantized domain, so n in and n out returns to the starting value for
-- ANY accepted step, not only for the ones that happened to be
-- representable.
local function effective_step()
  return quantize(pmacs.config.get("ui.gpu-zoom-step"))
end

local function effective_base()
  return quantize(pmacs.config.get("ui.gpu-font-size-base"))
end

-- The current size in logical px, or nil when the preference is unset.
-- nil is a REAL state (the frontend's own default), never inferred from
-- silence --- Q#TH7.
local function current_px()
  return pmacs.gpu.font().size
end

-- Preserve the configured family on EVERY write. `set_font` replaces
-- both fields unconditionally, so `set_font { size = n }` alone would
-- clear a family the user set in init.lua, and they would get it back
-- only by restarting.
local function write_size(px)
  local spec = { size = px }
  local family = pmacs.gpu.font().family
  if family then spec.family = family end
  pmacs.gpu.set_font(spec)
end

local function save(px)
  if not pmacs.state.available() then return end
  pmacs.state.write(STATE_KEY, string.format("%d\n", math.floor(px * 100 + 0.5)))
end

local function forget()
  if not pmacs.state.available() then return end
  pmacs.state.write(STATE_KEY, "")
end

-- Step by `delta` logical px. Returns the new size, or nil plus a
-- reason.
local function step(delta)
  local base = current_px() or effective_base()
  local want = quantize(base + delta)
  if want < MIN_PX or want > MAX_PX then
    -- Reject the WHOLE step rather than pinning to the boundary. This
    -- is what keeps "n steps in, n steps out returns exactly" true at
    -- the edges, which is precisely where a user steps back and forth.
    -- It also mirrors `apply_font_facts`, which rejects an
    -- out-of-range message outright rather than clamping it.
    return nil, string.format(
      "zoom: %.2f px is outside %.2f-%.2f; size unchanged", want, MIN_PX, MAX_PX)
  end
  write_size(want)
  save(want)
  return want
end

-- Named `increase`/`decrease` rather than `in`/`out`: `in` is a Lua
-- keyword, and `in_` reads like a workaround for one.
function pmacs.zoom.increase()
  return step(effective_step())
end

function pmacs.zoom.decrease()
  return step(-effective_step())
end

-- Reset returns the preference to NIL --- the frontend's own default ---
-- not to `ui.gpu-font-size-base`. The base is only the origin for a
-- first step; resetting to it would ship an explicit size that merely
-- happens to equal the default, making the untouched state unreachable
-- once a user has ever zoomed. Clearing the saved state too, because
-- "reset until restart" is not what the word says.
function pmacs.zoom.reset()
  local family = pmacs.gpu.font().family
  local spec = {}
  if family then spec.family = family end
  pmacs.gpu.set_font(spec)
  forget()
  return nil
end

-- Restore a saved zoom. Called from the Rust side AFTER
-- `install_state_dirs`, never at module load: builtins and init.lua both
-- run before state is wired up, so a read here would return nothing,
-- always. Zoom is this project's first EAGER state consumer --- saveplace
-- and recentf both read lazily inside functions and never meet this.
--
-- Whole-file parse, not a line iterator. `^(%d+)$` would reject the
-- newline-terminated file we write ourselves ($ anchors to end of
-- subject), and saveplace's `gmatch("([^\n]+)")` would accept the FIRST
-- line of a multi-line file --- fine for recentf, where a line is one
-- independent entry, wrong here, where the file IS the value.
function pmacs.zoom.restore()
  if not pmacs.state.available() then return nil end
  local text = pmacs.state.read(STATE_KEY)
  if not text then return nil end
  local centi = text:match("^(%d+)\n$")
  if not centi then return nil end
  centi = tonumber(centi)
  -- Range-check before it can reach `set_font`. A syntactically fine
  -- but out-of-range value --- hand-edited, or written by a future
  -- version with a wider range --- would otherwise be rejected as a
  -- whole message, leaving the user with neither the saved zoom nor an
  -- explanation.
  if centi < MIN_PX * 100 or centi > MAX_PX * 100 then return nil end
  local px = centi / 100
  write_size(px)
  return px
end

pmacs.command.define {
  name = "gpu.zoom-in",
  description = "Increase the GPU frontend's font size by one step",
  fn = function()
    local px, why = pmacs.zoom.increase()
    pmacs.editor.set_status(why or string.format("zoom: %.2f px", px))
  end,
}

pmacs.command.define {
  name = "gpu.zoom-out",
  description = "Decrease the GPU frontend's font size by one step",
  fn = function()
    local px, why = pmacs.zoom.decrease()
    pmacs.editor.set_status(why or string.format("zoom: %.2f px", px))
  end,
}

pmacs.command.define {
  name = "gpu.zoom-reset",
  description = "Return the GPU frontend to its own default font size",
  fn = function()
    pmacs.zoom.reset()
    pmacs.editor.set_status("zoom: reset to the frontend default")
  end,
}
