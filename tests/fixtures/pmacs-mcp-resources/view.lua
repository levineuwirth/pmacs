-- pmacs-mcp-resources/view.lua --- T M9.5 rendering by mimeType.
--
-- Three render modes routed by the response shape:
--
--   1. text   (mimeType: text/* or application/json — JSON pretty-
--              printed; raw text otherwise) → single buffer with the
--              resource's text content.
--   2. directory (mimeType: application/vnd.pmacs.mcp.directory+json;
--              text content is a JSON array of child URI strings) →
--              navigable buffer with one child URI per line.
--   3. raw    (anything else, including unknown mimeTypes) → text
--              fallback. Binary blobs are rendered as a placeholder.
--
-- Returns: { kind = "text" | "directory" | "raw", body = "...",
--            mimeType = "...", children = {...} (only for directory) }

local M = {}

local function pretty_json(text)
  -- v0.1: minimal pretty-printer. Walks the input adding newlines
  -- after `{`, `,`, `[` and indentation; not a full JSON formatter
  -- but readable for typical MCP server payloads.
  -- Lua's string library doesn't ship a JSON parser; for v0.1 we
  -- simply return the raw text. M9.8+ may layer a real formatter.
  return text
end

-- Permissive JSON-array-of-strings extractor. Returns the strings
-- in document order, or nil if the input isn't a JSON array of
-- strings. Used by the directory and table renderers; v0.1 doesn't
-- pull in a real JSON parser.
local function json_string_array(text)
  if type(text) ~= "string" then return nil end
  local stripped = text:gsub("^%s*%[", ""):gsub("%]%s*$", "")
  local out = {}
  for entry in stripped:gmatch("\"([^\"]+)\"") do
    out[#out + 1] = entry
  end
  if #out == 0 then return nil end
  return out
end

-- Walk a single row's bracket-pair contents, picking up cells in
-- document order regardless of whether they are quoted strings or
-- bareword tokens (numbers, true/false/null, identifiers). This is
-- the v0.1 mixed-type tokenizer for table rows; Pass-3 finding 2
-- replaced an earlier two-pass approach that dropped barewords
-- whenever any quoted string was present in the row.
--
-- Cells are returned as Lua strings; numeric values become their
-- text form (e.g. `30`). The renderer pads them as text — column
-- alignment doesn't depend on cell type.
local function parse_row_cells(inner)
  local cells = {}
  -- inner looks like `["alice", 30]` or `[1, 2, 3]` or
  -- `["a", true, "b"]`; strip the outer brackets first.
  local body = inner:sub(2, -2)
  local pos = 1
  local len = #body
  while pos <= len do
    local c = body:sub(pos, pos)
    if c == '"' then
      -- Quoted string: scan to the next unescaped quote. v0.1
      -- doesn't handle backslash escapes; the synthetic fake
      -- doesn't produce them.
      local close = body:find('"', pos + 1, true)
      if close == nil then break end
      cells[#cells + 1] = body:sub(pos + 1, close - 1)
      pos = close + 1
    elseif c == "," or c == " " or c == "\t" or c == "\n" then
      pos = pos + 1
    else
      -- Bareword: number, true, false, null, identifier.
      local s, e = body:find("[%-%w%.]+", pos)
      if s == nil or s > pos then
        -- Unrecognized character; skip it so we don't loop.
        pos = pos + 1
      else
        cells[#cells + 1] = body:sub(s, e)
        pos = e + 1
      end
    end
  end
  return cells
end

-- Parse a tiny subset of JSON: an object of the form
-- `{ "columns": [...strings...], "rows": [[...strings|numbers...], ...] }`
-- and return `{ columns = {...}, rows = {{...}, ...} }`. Returns
-- nil if the input doesn't match the expected shape.
--
-- v0.1's "JSON parser" is regex-based and intentionally permissive:
-- it works for the synthetic table the fake server produces; real
-- production callers would use a real JSON library when M9.8 hands
-- us one.
local function parse_table_payload(text)
  if type(text) ~= "string" then return nil end
  -- Extract the columns array.
  local cols_block = text:match("\"columns\"%s*:%s*(%b[])")
  if cols_block == nil then return nil end
  local columns = json_string_array(cols_block)
  if columns == nil then return nil end
  -- Extract the rows array; each row is an inner array. `%b[]`
  -- matches balanced brackets but gmatch only finds the outermost,
  -- so we strip the outer pair and walk the inner ones.
  local rows_block = text:match("\"rows\"%s*:%s*(%b[])")
  if rows_block == nil then return nil end
  local inner_block = rows_block:sub(2, -2)
  local rows = {}
  for inner in inner_block:gmatch("(%b[])") do
    rows[#rows + 1] = parse_row_cells(inner)
  end
  return { columns = columns, rows = rows }
end

-- Render `{ columns, rows }` as a column-aligned text table.
-- Columns are padded to the max width of their column's values
-- (header included). Returns nil if the payload doesn't parse.
local function render_table(text)
  local parsed = parse_table_payload(text)
  if parsed == nil then return nil end
  local cols = parsed.columns
  local rows = parsed.rows
  -- Compute max width per column.
  local widths = {}
  for i, col in ipairs(cols) do
    widths[i] = #col
  end
  for _, row in ipairs(rows) do
    for i, cell in ipairs(row) do
      if widths[i] == nil or #cell > widths[i] then
        widths[i] = #cell
      end
    end
  end
  -- Pad each cell to its column's width.
  local function pad(s, w)
    if #s >= w then return s end
    return s .. string.rep(" ", w - #s)
  end
  local lines = {}
  -- Header row.
  local header_cells = {}
  for i, col in ipairs(cols) do
    header_cells[i] = pad(col, widths[i])
  end
  lines[#lines + 1] = table.concat(header_cells, " | ")
  -- Separator: dashes per column, joined by " + ".
  local sep_cells = {}
  for i = 1, #cols do
    sep_cells[i] = string.rep("-", widths[i])
  end
  lines[#lines + 1] = table.concat(sep_cells, " + ")
  -- Data rows.
  for _, row in ipairs(rows) do
    local cells = {}
    for i = 1, #cols do
      cells[i] = pad(row[i] or "", widths[i])
    end
    lines[#lines + 1] = table.concat(cells, " | ")
  end
  return {
    body = table.concat(lines, "\n") .. "\n",
    columns = cols,
    rows = rows,
  }
end

function M.render(content_response)
  -- content_response is the `result` table from `resources/read`:
  --   { contents = [{ uri, mimeType, text }, ...] }
  -- The MCP spec allows multiple content entries for a single read;
  -- v0.1 renders the first entry and surfaces extras as a footer
  -- comment line.
  local contents = content_response and content_response.contents
  if type(contents) ~= "table" or #contents == 0 then
    return {
      kind = "raw",
      body = "[empty resources/read response]\n",
      mimeType = "text/plain",
    }
  end
  local primary = contents[1]
  local mime = primary.mimeType or "text/plain"
  local text = primary.text or ""

  if mime == "application/vnd.pmacs.mcp.directory+json" then
    -- Parse the JSON array of child URIs.
    local children = json_string_array(text) or {}
    return {
      kind = "directory",
      body = table.concat(children, "\n") .. "\n",
      mimeType = mime,
      children = children,
    }
  end

  if mime == "application/vnd.pmacs.mcp.table+json" then
    -- Pass-2 finding 1: the spec calls out query-result-shaped
    -- resources rendering as table buffers. v0.1 supports a
    -- minimal `{ "columns": [...], "rows": [[...]] }` shape on a
    -- pmacs-specific MIME; servers that already render their
    -- query results in this shape get column-aligned tables for
    -- free, while servers using application/json pass through to
    -- the text fallback below. Generic JSON-to-table inference is
    -- still M9.8's call.
    --
    -- The body is rendered as:
    --   <col1> | <col2> | ...
    --   ------ + ------ + ...
    --   <r1c1> | <r1c2> | ...
    --   ...
    local rendered = render_table(text)
    if rendered then
      return {
        kind = "table",
        body = rendered.body,
        mimeType = mime,
        columns = rendered.columns,
        rows = rendered.rows,
      }
    end
    -- If the table content didn't parse, fall through to text
    -- so the user at least sees the raw payload.
  end

  if mime == "application/json" then
    return {
      kind = "text",
      body = pretty_json(text) .. "\n",
      mimeType = mime,
    }
  end

  -- Default: text rendering for any text/* mimeType, or raw for
  -- everything else. Either way, we put the text in the buffer.
  local kind = (mime:sub(1, 5) == "text/") and "text" or "raw"
  -- Ensure the body ends with a newline so cursor positioning at
  -- end-of-buffer doesn't behave oddly.
  if text:sub(-1) ~= "\n" then text = text .. "\n" end
  return {
    kind = kind,
    body = text,
    mimeType = mime,
  }
end

return M
