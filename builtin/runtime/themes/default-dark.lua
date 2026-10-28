-- The `default-dark` theme (E10g.1): the look pmacs has always had, as a
-- named theme. Every entry equals `Theme::default_dark()`'s
-- (`src/highlight.rs`), which a row asserts name by name; the reasons
-- behind individual entries (why `constructor` reaches three grammars,
-- why `warning` is the loudest) are written there and not repeated here.
--
-- Syntax only, in palette indices 1 to 14: the TUI shows them in the
-- user's own terminal palette and the GPU through its xterm-like table.
-- It sets no `ui.*` face, so every surface keeps its frontend's default.
-- An index below 16 is its own fallback at both indexed depths, so no
-- entry needs a `fallback` field.

local function fg(c) return { fg = c } end
local function fg_bold(c) return { fg = c, bold = true } end
local function fg_italic(c) return { fg = c, italic = true } end

pmacs.theme.define("default-dark", {
  description = "The terminal's own palette for syntax; every surface its frontend's default",
  faces = {
    ["keyword"] = fg_bold(5),
    ["keyword.control"] = fg_bold(13),
    ["function"] = fg(4),
    ["function.method"] = fg(12),
    ["type"] = fg(3),
    ["type.builtin"] = fg(11),
    ["string"] = fg(2),
    ["comment"] = fg_italic(8),
    ["constant"] = fg(1),
    ["constant.builtin"] = fg(9),
    ["number"] = fg(1),
    ["operator"] = fg(6),
    ["variable"] = {},
    ["punctuation"] = {},
    -- LSP semantic-token types.
    ["macro"] = fg_bold(13),
    ["namespace"] = fg(11),
    ["parameter"] = { italic = true },
    ["property"] = fg(6),
    ["class"] = fg(3),
    ["struct"] = fg(3),
    ["enum"] = fg(3),
    ["interface"] = fg(3),
    ["enumMember"] = fg(1),
    ["modifier"] = fg_bold(5),
    ["decorator"] = fg(13),
    ["regexp"] = fg(2),
    ["typeParameter"] = fg_italic(11),
    -- Web grammars.
    ["tag"] = fg(5),
    ["attribute"] = fg(3),
    -- Lean 4, and the grammars its four captures also reach.
    ["constructor"] = fg(11),
    ["character"] = fg(2),
    ["keyword.conditional"] = fg_bold(13),
    ["warning"] = fg_bold(9),
    -- Markdown's strong and emphasis.
    ["markup.strong"] = { bold = true },
    ["markup.emphasis"] = { italic = true },
  },
})
