-- The default language servers; only the status hook of heir_init.lua.
-- Each tick: the gap since the last one when it exceeds 150 ms, and,
-- when it changes, what quit depends on and what the user is told.
local last
local gaps = io.open('@OUT@/gaps.txt', 'w')
local prev
pmacs.hook.add('process.after-tick', function()
  local now = pmacs.now_ms()
  if last and now - last > 150 then
    gaps:write(string.format('%d gap %d ms\n', now, now - last))
    gaps:flush()
  end
  last = now
  local errors = ''
  for _, id in ipairs(pmacs.buffer.list()) do
    local d = pmacs.describe.buffer(id)
    if d and d.name == '*errors*' then
      errors = id:slice(0, id:len()):gsub('\n', ' | ')
    end
  end
  local prompt = pmacs.minibuffer.is_active() and (pmacs.minibuffer.prompt() or '') or ''
  local servers = {}
  for _, info in ipairs(pmacs.lsp.list()) do
    servers[#servers + 1] = info.state and info.state.kind or '?'
  end
  local text = 'errors=' .. errors .. '\nprompt=' .. prompt .. '\nservers=' .. table.concat(servers, ',') .. '\n'
  if text ~= prev then
    prev = text
    local f = io.open('@OUT@/status.txt', 'a')
    f:write(string.format('t=%d\n%s', now, text))
    f:close()
  end
end)
