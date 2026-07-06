// lua_bindings/mcp.rs --- pmacs.mcp: MCP client surface (T M9.1).

//! `pmacs.mcp.*` — Model Context Protocol client bindings. Split out of
//! `lua_bindings.rs` verbatim (audit F-016); behavior unchanged. Reaches
//! the shared JSON converters (still physically in the `lsp` section of
//! `mod.rs`) via `super::`.

use std::cell::RefCell;
use std::rc::Rc;

use mlua::{FromLua, Lua, Table, UserData, UserDataMethods, Value};

use super::{SharedProcessSupervisor, json_to_lua, lua_to_json};

use crate::mcp::{
    McpClientState, McpError, McpEvent, McpEventKind, McpManager, McpRestartPolicy, McpServerId,
    McpServerSpec, SharedMcpManager, state_label_for as mcp_state_label_for,
};

/// Lua-facing wrapper around [`McpServerId`]. Mirrors
/// [`LspServerIdLua`].
#[derive(Copy, Clone)]
pub struct McpServerIdLua(pub McpServerId);

impl McpServerIdLua {
    /// The wrapped [`McpServerId`].
    #[must_use]
    pub fn id(self) -> McpServerId {
        self.0
    }
}

impl FromLua for McpServerIdLua {
    fn from_lua(value: Value, _: &Lua) -> mlua::Result<Self> {
        match value {
            Value::UserData(ud) => Ok(*ud.borrow::<Self>()?),
            other => Err(mlua::Error::FromLuaConversionError {
                from: other.type_name(),
                to: "McpServerIdLua".to_string(),
                message: Some("expected an MCP server handle".to_string()),
            }),
        }
    }
}

impl UserData for McpServerIdLua {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(mlua::MetaMethod::ToString, |_, this, ()| {
            Ok(format!("{}", this.0))
        });
        methods.add_meta_method(mlua::MetaMethod::Eq, |_, this, other: McpServerIdLua| {
            Ok(this.0 == other.0)
        });
        methods.add_method("raw", |_, this, ()| Ok(this.0.raw()));
    }
}

fn parse_mcp_restart(name: &str) -> mlua::Result<McpRestartPolicy> {
    Ok(match name {
        "never" | "Never" => McpRestartPolicy::Never,
        "on_crash" | "OnCrash" | "on-crash" => McpRestartPolicy::OnCrash,
        "always" | "Always" => McpRestartPolicy::Always,
        other => {
            return Err(mlua::Error::external(format!(
                "unknown MCP restart policy: {other:?} (expected never|on_crash|always)"
            )));
        }
    })
}

fn lua_to_mcp_spec(t: &Table) -> mlua::Result<McpServerSpec> {
    let label: String = t.get("label").unwrap_or_else(|_| "unnamed".to_owned());
    let command: String = t.get("command")?;
    let args: Vec<String> = t.get("args").unwrap_or_default();
    let cwd: Option<String> = t.get("cwd").ok().flatten();
    let env_t: Option<Table> = t.get("env").ok().flatten();
    let mut env: Vec<(String, String)> = Vec::new();
    if let Some(env_t) = env_t {
        env_t.for_each(|k: String, v: String| {
            env.push((k, v));
            Ok(())
        })?;
    }
    let restart = match t.get::<Option<String>>("restart").ok().flatten() {
        Some(s) => parse_mcp_restart(&s)?,
        None => McpRestartPolicy::OnCrash,
    };
    Ok(McpServerSpec {
        label,
        command,
        args,
        cwd: cwd.map(std::path::PathBuf::from),
        env,
        restart,
    })
}

fn mcp_state_to_lua(lua: &Lua, state: &McpClientState) -> mlua::Result<Table> {
    let t = lua.create_table_with_capacity(0, 5)?;
    t.set("kind", mcp_state_label_for(state))?;
    match state {
        McpClientState::Starting
        | McpClientState::Stopped { .. }
        | McpClientState::ShuttingDown => {}
        McpClientState::Initializing {
            init_request_id, ..
        } => {
            t.set("init_request_id", *init_request_id)?;
        }
        McpClientState::Initialized {
            capabilities,
            server_info,
            protocol_version,
            ..
        } => {
            t.set("capabilities", json_to_lua(lua, capabilities)?)?;
            if let Some(info) = server_info {
                t.set("server_info", json_to_lua(lua, info)?)?;
            }
            if let Some(v) = protocol_version {
                t.set("protocol_version", v.as_str())?;
            }
        }
        McpClientState::Crashed { reason, .. } => {
            t.set("reason", reason.as_str())?;
        }
    }
    Ok(t)
}

fn mcp_event_to_lua(lua: &Lua, ev: &McpEvent) -> mlua::Result<Table> {
    let t = lua.create_table_with_capacity(0, 5)?;
    t.set("server", McpServerIdLua(ev.server))?;
    match &ev.kind {
        McpEventKind::Started { pid } => {
            t.set("kind", "started")?;
            t.set("pid", *pid)?;
        }
        McpEventKind::Initialized { capabilities } => {
            t.set("kind", "initialized")?;
            t.set("capabilities", json_to_lua(lua, capabilities)?)?;
        }
        McpEventKind::Notification { method, params } => {
            t.set("kind", "notification")?;
            t.set("method", method.as_str())?;
            t.set("params", json_to_lua(lua, params)?)?;
        }
        McpEventKind::Request { id, method, params } => {
            t.set("kind", "request")?;
            t.set("request_id", json_to_lua(lua, id)?)?;
            t.set("method", method.as_str())?;
            t.set("params", json_to_lua(lua, params)?)?;
        }
        McpEventKind::Response {
            id,
            result,
            error,
            method,
        } => {
            t.set("kind", "response")?;
            t.set("request_id", *id)?;
            t.set("method", method.as_str())?;
            t.set("result", json_to_lua(lua, result)?)?;
            if let Some(e) = error {
                let err_t = lua.create_table_with_capacity(0, 3)?;
                err_t.set("code", e.code)?;
                err_t.set("message", e.message.as_str())?;
                if let Some(d) = &e.data {
                    err_t.set("data", json_to_lua(lua, d)?)?;
                }
                t.set("error", err_t)?;
            }
        }
        McpEventKind::ShuttingDown => {
            t.set("kind", "shutting_down")?;
        }
        McpEventKind::Stopped => {
            t.set("kind", "stopped")?;
        }
        McpEventKind::Crashed { reason } => {
            t.set("kind", "crashed")?;
            t.set("reason", reason.as_str())?;
        }
        McpEventKind::Restarting { attempt } => {
            t.set("kind", "restarting")?;
            t.set("attempt", *attempt)?;
        }
        McpEventKind::Stderr(bytes) => {
            t.set("kind", "stderr")?;
            t.set("bytes", lua.create_string(bytes)?)?;
        }
        McpEventKind::ProtocolError { message } => {
            t.set("kind", "protocol_error")?;
            t.set("message", message.as_str())?;
        }
    }
    Ok(t)
}

/// Lua → [`McpError`]. Either a plain string (becomes a generic
/// `code = -32603` internal error) or a table with `{code, message,
/// data?}`.
fn lua_to_mcp_error(value: Value) -> mlua::Result<McpError> {
    match value {
        Value::String(s) => Ok(McpError {
            code: -32603,
            message: s.to_str()?.to_owned(),
            data: None,
        }),
        Value::Table(t) => {
            let code: i64 = t.get("code").unwrap_or(-32603);
            let message: String = t.get("message").unwrap_or_else(|_| "error".to_owned());
            let data = match t.get::<Option<Value>>("data")? {
                Some(Value::Nil) | None => None,
                Some(other) => Some(lua_to_json(other)?),
            };
            Ok(McpError {
                code,
                message,
                data,
            })
        }
        other => Err(mlua::Error::external(format!(
            "mcp error must be a string or table, got {}",
            other.type_name()
        ))),
    }
}

/// Install `pmacs.mcp.*` (T M9.1).
///
/// Function inventory (10 entries; each parallels the same-named
/// `pmacs.lsp.*` entry, with the single addition of `capabilities`
/// for M9.1's "discoverable through the worker" acceptance bullet):
///
/// | `pmacs.mcp.*`        | `pmacs.lsp.*`        | Notes                                   |
/// |----------------------|----------------------|-----------------------------------------|
/// | `spawn`              | `spawn`              | Same shape; MCP spec drops `language_id`/`root_uri`. |
/// | `stop`               | `stop`               | Identical.                              |
/// | `send_request`       | `send_request`       | Identical (JSON-RPC 2.0 layer is same). |
/// | `send_notification`  | `send_notification`  | Identical.                              |
/// | `send_response`      | `send_response`      | Identical.                              |
/// | `events_take`        | `events_take`        | Identical.                              |
/// | `list`               | `list`               | Identical.                              |
/// | `forget`             | `forget`             | Identical.                              |
/// | `_tick`              | `_tick`              | Identical.                              |
/// | `capabilities`       | (none — folded into `list`/`status_summary`) | M9.1 acceptance: `pmacs.mcp.capabilities(id)` returns the server's declared capabilities table or nil. |
#[allow(
    clippy::too_many_lines,
    reason = "linear list of raw bindings; splitting fragments a coherent surface"
)]
pub fn install_mcp(lua: &Lua, manager: &SharedMcpManager) -> mlua::Result<()> {
    lua.set_app_data(manager.clone());
    let pmacs: Table = lua.globals().get("pmacs")?;
    let mcp_mod = lua.create_table()?;

    {
        let m = manager.clone();
        mcp_mod.set(
            "spawn",
            lua.create_function(move |_, spec: Table| {
                let parsed = lua_to_mcp_spec(&spec)?;
                let id = m
                    .borrow_mut()
                    .spawn(parsed)
                    .map_err(mlua::Error::external)?;
                Ok(McpServerIdLua(id))
            })?,
        )?;
    }

    {
        let m = manager.clone();
        mcp_mod.set(
            "stop",
            lua.create_function(move |_, id: McpServerIdLua| {
                m.borrow_mut().stop(id.0).map_err(mlua::Error::external)?;
                Ok(())
            })?,
        )?;
    }

    {
        // Raw MCP request dispatcher. Returns the async-runtime
        // [`JobId`] the response will settle. Package code does not
        // call this directly — `builtin/runtime/mcp.lua` wraps it
        // into a Handle-returning `pmacs.mcp.send_request` that
        // matches the dispatch shape of `pmacs.workers.compute_sum`,
        // `pmacs.fs.read_dir`, etc. The underscore-prefixed name
        // marks this as the unwrapped primitive (mirrors
        // `pmacs._async._dispatch_*` in async.lua).
        let m = manager.clone();
        mcp_mod.set(
            "_send_request_raw",
            lua.create_function(
                move |_, (id, method, params): (McpServerIdLua, String, Option<Value>)| {
                    let json_params = match params {
                        Some(Value::Nil) | None => serde_json::Value::Null,
                        Some(other) => lua_to_json(other)?,
                    };
                    let job_id = m
                        .borrow_mut()
                        .send_request(id.0, method, json_params)
                        .map_err(mlua::Error::external)?;
                    Ok(job_id)
                },
            )?,
        )?;
    }

    {
        let m = manager.clone();
        mcp_mod.set(
            "send_notification",
            lua.create_function(
                move |_, (id, method, params): (McpServerIdLua, String, Option<Value>)| {
                    let json_params = match params {
                        Some(Value::Nil) | None => serde_json::Value::Null,
                        Some(other) => lua_to_json(other)?,
                    };
                    m.borrow_mut()
                        .send_notification(id.0, method, json_params)
                        .map_err(mlua::Error::external)?;
                    Ok(())
                },
            )?,
        )?;
    }

    {
        let m = manager.clone();
        mcp_mod.set(
            "send_response",
            lua.create_function(
                move |_,
                      (id, request_id, result, err): (
                    McpServerIdLua,
                    Value,
                    Value,
                    Option<Value>,
                )| {
                    let request_id_json = lua_to_json(request_id)?;
                    let outcome = match err {
                        Some(Value::Nil) | None => Ok(lua_to_json(result)?),
                        Some(e) => Err(lua_to_mcp_error(e)?),
                    };
                    m.borrow_mut()
                        .send_response(id.0, request_id_json, outcome)
                        .map_err(mlua::Error::external)?;
                    Ok(())
                },
            )?,
        )?;
    }

    {
        let m = manager.clone();
        mcp_mod.set(
            "events_take",
            lua.create_function(move |lua, id: McpServerIdLua| {
                let evs = m.borrow_mut().take_events(id.0);
                let out = lua.create_table_with_capacity(evs.len(), 0)?;
                for (i, ev) in evs.iter().enumerate() {
                    out.set(i + 1, mcp_event_to_lua(lua, ev)?)?;
                }
                Ok(out)
            })?,
        )?;
    }

    {
        let m = manager.clone();
        mcp_mod.set(
            "list",
            lua.create_function(move |lua, ()| {
                let mgr = m.borrow();
                let ids: Vec<McpServerId> = mgr.ids().collect();
                let out = lua.create_table_with_capacity(ids.len(), 0)?;
                for (i, id) in ids.iter().enumerate() {
                    let row = lua.create_table_with_capacity(0, 4)?;
                    row.set("id", McpServerIdLua(*id))?;
                    if let Some(spec) = mgr.spec(*id) {
                        row.set("label", spec.label.as_str())?;
                        row.set("command", spec.command.as_str())?;
                    }
                    if let Some(state) = mgr.state(*id) {
                        row.set("state", mcp_state_to_lua(lua, state)?)?;
                    }
                    if let Some(attempt) = mgr.attempt(*id) {
                        row.set("attempt", attempt)?;
                    }
                    out.set(i + 1, row)?;
                }
                Ok(out)
            })?,
        )?;
    }

    {
        let m = manager.clone();
        mcp_mod.set(
            "forget",
            lua.create_function(move |_, id: McpServerIdLua| {
                m.borrow_mut().forget(id.0).map_err(mlua::Error::external)?;
                Ok(())
            })?,
        )?;
    }

    {
        let m = manager.clone();
        mcp_mod.set(
            "_tick",
            lua.create_function(move |_, ()| {
                m.borrow_mut().tick();
                Ok(())
            })?,
        )?;
    }

    {
        // M9.1 acceptance: declared capabilities are discoverable
        // through the worker. Returns the server's `capabilities`
        // table (as JSON-translated Lua) or `nil` if the server is
        // not yet initialized.
        let m = manager.clone();
        mcp_mod.set(
            "capabilities",
            lua.create_function(move |lua, id: McpServerIdLua| {
                let mgr_ref = m.borrow();
                match mgr_ref.capabilities(id.0) {
                    Some(caps) => json_to_lua(lua, caps),
                    None => Ok(Value::Nil),
                }
            })?,
        )?;
    }

    {
        // T M9.2: raw resource-read dispatcher. Returns the
        // async-runtime [`JobId`] the response will settle.
        // `builtin/runtime/mcp.lua` wraps it into a Handle-returning
        // `pmacs.mcp.read_resource(server, uri)`.
        let m = manager.clone();
        mcp_mod.set(
            "_read_resource_raw",
            lua.create_function(move |_, (id, uri): (McpServerIdLua, String)| {
                let job_id = m
                    .borrow_mut()
                    .read_resource(id.0, uri)
                    .map_err(mlua::Error::external)?;
                Ok(job_id)
            })?,
        )?;
    }

    {
        // T M9.2: explicit cache invalidation. Drops the per-(server,
        // uri) cache entry; subsequent `read_resource` calls
        // re-dispatch. In-flight requests at the moment of
        // invalidation still settle their awaiters with the arriving
        // result, but do not re-cache.
        let m = manager.clone();
        mcp_mod.set(
            "invalidate_resource",
            lua.create_function(move |_, (id, uri): (McpServerIdLua, String)| {
                m.borrow_mut().invalidate_resource(id.0, uri);
                Ok(())
            })?,
        )?;
    }

    {
        // T M9.3: raw tool dispatcher. Returns the async-runtime
        // [`JobId`] the response will settle. `builtin/runtime/mcp.lua`
        // wraps it into a Handle-returning
        // `pmacs.mcp.invoke_tool(server, name, args)`.
        let m = manager.clone();
        mcp_mod.set(
            "_invoke_tool_raw",
            lua.create_function(
                move |_, (id, name, args): (McpServerIdLua, String, Option<Value>)| {
                    let json_args = match args {
                        Some(Value::Nil) | None => {
                            serde_json::Value::Object(serde_json::Map::default())
                        }
                        Some(other) => lua_to_json(other)?,
                    };
                    let job_id = m
                        .borrow_mut()
                        .invoke_tool(id.0, name, json_args)
                        .map_err(mlua::Error::external)?;
                    Ok(job_id)
                },
            )?,
        )?;
    }

    {
        // T M9.5: register interest in a JSON-RPC notification method.
        // The notification dispatcher in `builtin/runtime/mcp.lua`
        // calls this when the first handler for a method is added,
        // and `_unsubscribe_notification` when the last one drops.
        let m = manager.clone();
        mcp_mod.set(
            "_subscribe_notification",
            lua.create_function(move |_, method: String| {
                m.borrow_mut().subscribe_notification(method);
                Ok(())
            })?,
        )?;
    }

    {
        let m = manager.clone();
        mcp_mod.set(
            "_unsubscribe_notification",
            lua.create_function(move |_, method: String| {
                m.borrow_mut().unsubscribe_notification(&method);
                Ok(())
            })?,
        )?;
    }

    {
        // T M9.5: drain all queued subscribed notifications. Returns
        // a Lua table { [method] = { { server = ..., params = ... }, ... } }.
        // The Lua tick hook walks this and invokes registered
        // handlers.
        let m = manager.clone();
        mcp_mod.set(
            "_drain_notifications",
            lua.create_function(move |lua, ()| {
                let drained = m.borrow_mut().drain_notifications();
                let out = lua.create_table_with_capacity(0, drained.len())?;
                for (method, entries) in drained {
                    let arr = lua.create_table_with_capacity(entries.len(), 0)?;
                    for (i, (sid, params)) in entries.into_iter().enumerate() {
                        let entry = lua.create_table_with_capacity(0, 2)?;
                        entry.set("server", McpServerIdLua(sid))?;
                        entry.set("params", json_to_lua(lua, &params)?)?;
                        arr.set(i + 1, entry)?;
                    }
                    out.set(method.as_str(), arr)?;
                }
                Ok(out)
            })?,
        )?;
    }

    {
        // T M9.4: raw prompt dispatcher. Returns the async-runtime
        // [`JobId`] the response will settle. `builtin/runtime/mcp.lua`
        // wraps it into a Handle-returning
        // `pmacs.mcp.get_prompt(server, name, args)`.
        //
        // `args` of `nil` (or omitted) translates to an empty object
        // on the wire — the MCP spec requires the `arguments` field
        // even for prompts that take no arguments. Three Lua call
        // patterns produce identical wire requests:
        //   pmacs.mcp.get_prompt(s, "p")        (no third arg)
        //   pmacs.mcp.get_prompt(s, "p", nil)   (explicit nil)
        //   pmacs.mcp.get_prompt(s, "p", {})    (empty table)
        let m = manager.clone();
        mcp_mod.set(
            "_get_prompt_raw",
            lua.create_function(
                move |_, (id, name, args): (McpServerIdLua, String, Option<Value>)| {
                    let json_args = match args {
                        Some(Value::Nil) | None => {
                            serde_json::Value::Object(serde_json::Map::default())
                        }
                        Some(other) => lua_to_json(other)?,
                    };
                    let job_id = m
                        .borrow_mut()
                        .get_prompt(id.0, name, json_args)
                        .map_err(mlua::Error::external)?;
                    Ok(job_id)
                },
            )?,
        )?;
    }

    pmacs.set("mcp", mcp_mod)?;
    Ok(())
}

/// Build a fresh [`McpManager`] over `supervisor` and install
/// `pmacs.mcp.*` over it. Mirrors [`make_lsp_manager`] in shape;
/// also takes the editor's [`SharedAsyncRuntime`] so MCP responses
/// settle Lua-visible job ids without occupying a worker thread
/// (T M9.1, Pass-2 finding 1).
pub fn make_mcp_manager(
    lua: &Lua,
    supervisor: SharedProcessSupervisor,
    runtime: crate::async_runtime::SharedAsyncRuntime,
) -> mlua::Result<SharedMcpManager> {
    let manager = Rc::new(RefCell::new(McpManager::new(supervisor, runtime)));
    install_mcp(lua, &manager)?;
    Ok(manager)
}
