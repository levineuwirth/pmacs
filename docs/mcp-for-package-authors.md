# MCP for package authors

This guide is for package authors who want to integrate Model
Context Protocol (MCP) servers into pmacs. It covers the
`pmacs.mcp.*` API, the architectural pattern for AI-assistance
packages, and the disciplines distilled from M9.5 – M9.8's audit
work.

The general package-publishing mechanics are at
[`docs/package-author-guide.md`](package-author-guide.md). This
document is the inverse: how to write a package that *uses* an
MCP server.

Five fixture packages live under `tests/fixtures/` as worked
examples:

- [`pmacs-mcp-resources/`](../tests/fixtures/pmacs-mcp-resources/) — resources
- [`pmacs-mcp-tools/`](../tests/fixtures/pmacs-mcp-tools/) — tools-as-commands
- [`pmacs-mcp-prompts/`](../tests/fixtures/pmacs-mcp-prompts/) — prompts-as-result-buffers
- [`pmacs-mcp-ai/`](../tests/fixtures/pmacs-mcp-ai/) — AI-assistance composing the above

---

## 1. Why MCP for package authors

The architectural claim (`spec/pmacs-spec.tex`, §sec:m9-ai) is:

> AI is a transport binding, not a feature.

In practical terms: pmacs has no built-in "ask Claude" command, no
hard-coded model API, no Anthropic-specific or OpenAI-specific code
anywhere in the editor. Instead, pmacs ships an MCP transport
layer, and AI features are built as packages on top.

This means:

1. **Your package speaks MCP, not a model API.** The model behind
   the configured server is interchangeable. Re-pointing your
   package at a different MCP server changes which model serves
   the prompts; your code is unchanged.
2. **Your package composes with other MCP packages.** The
   transport layer is shared; subscriptions, caching, cancellation
   are uniform. Two packages talking to the same server share a
   single process.
3. **The user's API keys live in the MCP server's environment, not
   yours.** Your package never sees credentials. The server
   process inherits them; pmacs's spawn API takes a `command` and
   optional `env`.

The recommendation: **write against the MCP layer, not against
any specific model API.** If your package wants to talk to Claude,
spawn an MCP server that talks to Claude. If your package wants to
talk to GPT, spawn an MCP server that talks to GPT. Your package
code is the same.

---

## 2. The transport: `pmacs.mcp.*`

The full public API surface is six Lua functions plus userdata
methods. Everything else MCP-related is reachable from these.

### `pmacs.mcp.spawn { ... }` → `McpServerIdLua`

Spawn an MCP server as a child process and start the initialize
handshake.

```lua
local server = pmacs.mcp.spawn {
  label   = "my-server",       -- string. Used as buffer-name prefix and roster key.
  command = "/usr/local/bin/mcp-claude",
  args    = { "--config", "/path/to/config.toml" },  -- optional
  env     = { ANTHROPIC_API_KEY = os.getenv("ANTHROPIC_API_KEY") },  -- optional
  restart = "OnCrash",         -- "OnCrash" (default) | "Always" | "Never"
}
```

Spawning is *asynchronous*: `spawn` returns immediately with a
handle. The server may take some time to initialize. Use
`pmacs.mcp.list()` to observe state transitions.

### `pmacs.mcp.list()` → array

Returns the current server roster:

```lua
for _, row in ipairs(pmacs.mcp.list()) do
  print(row.label, row.id, row.state.kind)
  -- row.state.kind: "spawning" | "initializing" | "initialized" | "crashed" | "exited"
end
```

Use `state.kind == "initialized"` as the gate for sending requests.
Sending to a non-initialized server raises `not ready for
requests`.

### `pmacs.mcp.read_resource(server, uri)` → handle

Read an MCP resource. Returns an *awaitable handle*:

```lua
pmacs.async(function()
  local body = pmacs.mcp.read_resource(server, "file:///etc/config.toml"):await()
  -- body = { contents = [ { uri, text? | blob?, mimeType? }, ... ] }
end)
```

The handle is *cache-aware*: a settled response is returned from
cache for subsequent calls with the same `(server, uri)` until
invalidation. Concurrent calls during an in-flight request share
the awaitable. Cancellation of an awaiter is independent — the
wire request is only cancelled when *all* awaiters cancel.

Invalidation triggers:

- `notifications/resources/updated` (per-uri) — invalidates that uri.
- `notifications/resources/list_changed` — invalidates all
  resources for that server.

You don't typically call `on_notification` for these — the cache
listens internally.

### `pmacs.mcp.invoke_tool(server, name, args)` → handle

Invoke an MCP tool. Returns an awaitable handle:

```lua
pmacs.async(function()
  local result = pmacs.mcp.invoke_tool(server, "search", { query = "foo" }):await()
  -- result = { content = [ { type, text? | image? | resource? }, ... ],
  --            isError = bool, _meta? }
  if result.isError then
    -- semantic failure: the tool ran but reported a failure
  end
end)
```

**Three failure modes** to distinguish:

1. **Success**: `isError = false`, content has the result.
2. **Semantic failure**: `isError = true`, content describes the
   failure ("file not found", "permission denied"). The tool
   ran; the operation failed.
3. **Transport / protocol failure**: the `:await()` call raises.
   Either the server is gone, the request was cancelled, the
   server returned a JSON-RPC error (unknown tool, invalid args).

Don't conflate (2) and (3). Semantic failures are *results*;
transport failures are *exceptions*.

Tool calls are not cached client-side — the server may have side
effects, and v0.1 doesn't read the MCP idempotency hint.
Cancellation is via `:cancel()` on the handle.

### `pmacs.mcp.get_prompt(server, name, args)` → handle

Get an MCP prompt response. Returns an awaitable handle:

```lua
pmacs.async(function()
  local response = pmacs.mcp.get_prompt(server, "review_function", {
    language = "rust",
    file_path = "src/main.rs",
    source = "fn main() { ... }",
  }):await()
  -- response = { description?, _meta?, messages = [ { role, content }, ... ] }
end)
```

Required arguments are validated by the server — missing them
raises a JSON-RPC error.

`response._meta.format` carries a content-type hint when the
server supports it: `"text"` (default), `"code"` (with
`_meta.language` for syntax highlighting), `"markdown"`. Unknown
formats fall back to `text`. The `pmacs-mcp-prompts.render`
function (see §6) reads these hints and routes the buffer through
the appropriate highlight pipeline.

`args` may be a Lua table with structured values — arrays, nested
objects. The wire shape is JSON. M9.8's `pmacs-mcp-ai` uses this
to send a structured `files: [{path, content}, ...]` array for
its project-context prompt. Don't separator-encode JSON into
strings; let the marshaler handle it.

### `pmacs.mcp.on_notification(method, fn)` → token

Subscribe to MCP server-to-client notifications. The subscription
is **global per method**, not per-server: the handler `fn` receives
`(server, params)` and is responsible for filtering by `server`
if it cares.

```lua
local token = pmacs.mcp.on_notification("notifications/tools/list_changed", function(server, params)
  -- This fires for *any* server's list_changed. Filter if you only
  -- want events from servers your package has registered:
  if not _registered_servers[server:raw()] then return end
  -- re-fetch tools/list and reconcile commands
end)

-- Later:
pmacs.mcp.off_notification("notifications/tools/list_changed", token)
```

Multiple subscriptions to the same method fire in registration
order. A throwing callback doesn't break the dispatcher — the
error is logged via `pmacs.error` and the next callback fires.

The dispatcher under the hood is a single per-tick drain regardless
of how many packages have registered. The Rust side is told to
queue notifications for `method` on the first subscription and
stop on the last unsubscription, so there's no idle-cost when no
package cares about a method.

**Subscription discipline**: balance subscribes with cancels.
M9.6's per-server-refcount finding showed how easily a re-register
flow leaks subscriptions: if your package's `register(server)` calls
`on_notification` unconditionally, registering N servers leaks N-1
subscriptions for the same method. Use a per-server refcount —
add the subscription on the *first* server registered, drop it on
the *last* server unregistered.

---

## 3. Server lifecycle in your package

The shape M9.6/M9.7/M9.8 settled on:

```lua
local M = {}

local _registered_servers = {}  -- per-server state, keyed by server:raw()

function M.register(server)
  local key = server:raw()
  if _registered_servers[key] then return end  -- idempotent
  _registered_servers[key] = {
    -- ... per-server state: subscriptions, label, etc.
  }
  -- subscribe, fetch initial state, define commands, etc.
end

function M.unregister(server)
  local key = server:raw()
  local state = _registered_servers[key]
  if state == nil then return end
  -- unsubscribe, drop commands, etc.
  _registered_servers[key] = nil
end
```

Why `server:raw()` and not `tostring(server)`: M9.6 finding 2.
The `:raw()` userdata method returns the canonical underlying id
as a string; `tostring` formats it for display and may not be
stable across pmacs versions.

### Server-gone teardown

When the server crashes or exits, in-flight requests fail with
`unknown server` or `not ready for requests`. Detect these in
your dispatch path:

```lua
local function looks_like_server_gone(err)
  local s = type(err) == "table" and tostring(err.message or "") or tostring(err)
  return s:find("unknown server", 1, true) ~= nil
      or s:find("not ready for requests", 1, true) ~= nil
end
```

When this fires, tear down your per-server state (drop commands,
clear caches) so the next user invocation surfaces a useful
"reconfigure" message rather than the same dead-server error on
every retry. M9.6 finding 5.

---

## 4. Pattern: tools as commands

Pattern from `pmacs-mcp-tools`. Each tool advertised by the
server becomes a command at `<label>-<tool-name>`. The list of
advertised tools is fetched via the generic `pmacs.mcp.send_request`
seam — there's no dedicated `pmacs.mcp.list_tools` in v0.1
because every server's `tools/list` shape is identical and the
generic seam handles it cleanly:

```lua
function M.register(server)
  pmacs.async(function()
    local result = pmacs.mcp.send_request(server, "tools/list", {}):await()
    -- result.tools is the array of tool entries: { name, description?, inputSchema? }
    for _, tool in ipairs(result.tools or {}) do
      define_command_for(server, tool)
    end
  end)
end
```

`pmacs.mcp.send_request(server, method, params)` returns the same
awaitable handle shape as `read_resource` / `invoke_tool` /
`get_prompt` and is the bottom-rung public seam: any MCP request
the spec defines is reachable through it. The
`pmacs-mcp-tools/init.lua` fixture is the worked example.

(Whether `tools/list` and `prompts/list` deserve *dedicated* typed
surfaces is an audit consideration; v0.2 territory if real package
authors find the generic seam awkward.)

### Reconciliation on `list_changed`

Tools can change at runtime. Subscribe (global per-method; filter
by server inside the handler):

```lua
pmacs.mcp.on_notification("notifications/tools/list_changed", function(server, params)
  if not _registered_servers[server:raw()] then return end
  -- Re-fetch tools/list, diff against current commands, add/remove.
end)
```

Compute a *schema hash* per tool to detect schema changes (not
just name changes). Add commands for new tools, remove for gone
tools, redefine for changed-schema tools. M9.6's package shows
the canonical implementation.

### Cross-source command collisions

Two servers advertising a tool with the same name can both want
to register `<label>-<tool-name>` if the labels collide, or
different name shapes can collide with builtins. Always check:

```lua
if pmacs.command.exists(name) then
  -- skip + warn, don't abort the rest of the registration
else
  pmacs.command.define { name = name, ... }
end
```

M9.6 finding 6.

---

## 5. Pattern: prompts as result buffers

Pattern from `pmacs-mcp-prompts`. Each prompt becomes a command
that prompts for required args, calls `get_prompt`, renders the
response into a `*mcp:<label>:<prompt>*` buffer.

The package exposes a public function that v0.2+ packages should
*compose with*, not duplicate:

```lua
local mcp_prompts = require("pmacs-mcp-prompts")

-- After calling pmacs.mcp.get_prompt and awaiting:
mcp_prompts.render(server_label, prompt_name, response)
```

`render` handles:

- buffer creation / reuse (keyed by `(label, prompt)`)
- read-only intercept (so the user can't accidentally edit the
  result)
- format dispatch via `_meta.format` (text / code / markdown)
- syntax highlighting attach (for `code` / `markdown` formats)
- cursor / region / scroll reset on re-paint

If your package wants different rendering — multi-turn
conversation history, inline image rendering, etc. — you write
your own. The composition story is *opt-in*: M9.8 chose to
compose so that re-invoking the same prompt from M9.7's auto-
registered command and M9.8's `ai.ask-about-X` lands in the same
buffer. Your package may have different ergonomic goals.

---

## 6. Pattern: AI assistance composing the above

Pattern from `pmacs-mcp-ai`. The full M9.8 example is 247 lines
of code (461 total with comments). Three commands, tree-sitter
context selection, project-buffer collection, structured-arg
prompts.

The architectural commitment, recorded in `init.lua`'s header:

```
-- Architectural commitment (the M9.8 ship gate):
--
--   * Zero direct calls into the Rust core. Everything reaches the
--     Rust side through the public Lua surface (`pmacs.mcp.*`,
--     `pmacs.parse.*`, `pmacs.command.*`, etc.).
--   * Zero model-specific code. The package speaks MCP; the model
--     behind the configured server is interchangeable.
```

The `configure { server_label, prompts = { ... } }` shape is the
key:

```lua
local ai = require("pmacs-mcp-ai")
ai.configure {
  server_label = "claude-mcp",
  prompts = {
    fn      = "review_function",
    project = "review_project",
    ask     = "ask_freeform",
  },
}
```

`server_label` is the *only* model-specific input. Re-configure
to a different `server_label` (with the same prompt names served
by a different MCP server) and the same commands route to the
new server. Zero code changes.

This is the recommendation in concrete form: **structure your
package's API so that the model is a configuration knob, not
a code path.**

---

## 7. Disciplines from the M9 audits

The audit findings from M9.6 – M9.8 distill into a small set of
disciplines worth applying up-front in every new MCP package:

### `notify()` should hit both status and error

Status messages are overwritten by the next status message.
Errors persist in `*pmacs-error*`. Use both:

```lua
local function notify(msg)
  pmacs.editor.set_status(msg)
  if pmacs.error then
    pmacs.error("my-package: " .. msg)
  end
end
```

M9.6 finding 10.

### Server-gone clears your local state

See §3.

### Cross-source DuplicateName via `pmacs.command.exists`

See §4.

### Subscription refcount

If your `register` flow subscribes to notifications, balance
subscribes with cancels. Use a per-server refcount, not "always
subscribe."

M9.6 finding 3.

### Required-arg-order is identity, not order

When validating prompts/tools args, treat the `arguments` field
as a *set* of required names. The advertised order is for UI
prompting; required-ness is per-arg, not per-position.

M9.6 finding 4.

### Buffer state keyed by `tostring(buf)`, not userdata

When tracking per-buffer state in a Lua table:

```lua
-- WRONG: silently fails on every re-lookup
local _state = {}
_state[buf] = { ... }

-- RIGHT: stable per underlying BufferId
local _state = {}
_state[tostring(buf)] = { ... }
```

`pmacs.buffer.list()` and `pmacs.window.buffer()` return *fresh*
userdata wrappings on every call — a userdata-keyed lookup only
finds the first wrapping ever inserted. `tostring(buf)` is
stable per underlying id, same convention
`builtin/runtime/syntax.lua` already uses for
`highlighted_buffers`.

M9.8 amendment to M9.7 audit. The bug went undetected through
M9.7's full acceptance suite because the suite only checked
buffer *count*, not body update.

### Test seams use `_underscore_prefix`

Functions exposed on your module table for tests but not part of
your stable public API should be `_prefix`-named:

```lua
function M._find_enclosing_function(buf, byte_pos)
  -- Test seam (unstable). Public API uses the higher-level
  -- ai.ask-about-function command.
end
```

This is an idiom, not enforced — but it makes the audit's
"public API surface" math unambiguous. The M9.6 audit's
`_render_schema_doc`, M9.7's `_format_messages`, M9.8's
`_collect_project_files` all follow this pattern.

---

## 8. The five fixture packages as worked examples

| Package | Demonstrates |
|---------|--------------|
| `pmacs-mcp-resources` | Resource read with cache awareness; `notifications/resources/updated` consumption |
| `pmacs-mcp-tools` | Tool-call dispatch; tools-as-commands reconciliation on `list_changed`; M9.6's twelve audit findings disposed |
| `pmacs-mcp-prompts` | Prompt-as-result-buffer rendering; format-hint dispatch; tree-sitter highlight attach for code/markdown |
| `pmacs-mcp-ai` | AI-assistance composing the above; tree-sitter context selection; structured-arg prompts; server pluggability |

Each ships under `tests/fixtures/` because they're audit
fixtures. The packages compile and run as real packages — the
fixture location is just where they live in-tree. v0.2+ may move
them to `builtin/packages/` once the audit pipeline is settled.

The corresponding audit docs are `M9.5-AUDIT.md` ...
`M9.9-AUDIT.md`. Each documents the per-package architectural
decisions, surface-area math, and audit-finding disposition.

---

## 9. Where things go wrong

Common pitfalls:

1. **Sending requests before `state.kind == "initialized"`**.
   `pmacs.mcp.spawn` returns before the handshake completes. Gate
   on the roster row's state.kind, or use the wait-and-poll
   pattern from `tests/m9_8_acceptance.rs`'s
   `spawn_initialized_server`.

2. **Not distinguishing `isError` from JSON-RPC error**. A tool
   reporting `isError = true` is a normal result. Don't `pcall`
   the await and treat all failures equally — the user wants
   different UX for "the tool said no" vs "the server fell over."

3. **Caching tool results client-side**. Don't. v0.1's tool layer
   doesn't, and you shouldn't either — tools may have side
   effects, and the MCP idempotency hint isn't surfaced yet.

4. **Subscribing per-call instead of per-server**. If your
   command subscribes to `list_changed` every invocation, you'll
   accumulate subscriptions. Subscribe at `register` time,
   unsubscribe at `unregister` time, refcount across servers.

5. **Reaching past `pmacs.mcp.*` to a transport detail**. If you
   find yourself wanting `pmacs.mcp._send_raw_jsonrpc` or similar,
   stop and ask whether your use case warrants a public API
   addition. The audit pattern is: surface real use cases, then
   promote when a second consumer materializes (the
   "promote-on-second-consumer" discipline). Don't reach around.

---

## 10. Versioning and stability

The `pmacs.mcp.*` Lua API is the stable surface. Six functions
plus userdata methods. The shape is locked for v1.0; additions
are backwards-compatible.

The fixture packages are *fixtures*: their public shape is
stable enough for examples to keep working, but minor versions
may add fields to response shapes (e.g., M9.7 added
`_meta.format` honoring; M9.8 amendment promoted `M.render`).
Treat their documented public functions as semver-respecting;
treat their `_underscore_prefix` test seams as unstable.

The MCP protocol itself versions independently. pmacs's transport
layer reads `protocolVersion` from the initialize result and
rejects unsupported versions (M9 Pass-2 finding 3). Your package
doesn't need to know about protocol versions — pmacs handles that.

---

## See also

- [`docs/package-author-guide.md`](package-author-guide.md) — general
  package mechanics (manifest, addresses, lockfiles, audit lint)
- [`TRANSITION-M9.md`](../TRANSITION-M9.md) — M9 milestone summary,
  deferred items, audit-finding accumulation
- [`spec/pmacs-spec.tex` §sec:m9-ai](../spec/pmacs-spec.tex) — the
  architectural claim *AI is a transport binding, not a feature*
- `tests/fixtures/pmacs-mcp-{resources,tools,prompts,ai}/init.lua`
  — the four worked examples
