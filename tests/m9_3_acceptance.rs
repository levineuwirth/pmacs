// m9_3_acceptance.rs --- T M9.3 tool invocation acceptance.

//! Acceptance tests for T M9.3 (`spec/pmacs-tasks.tex:3931`):
//!
//!   1. Tool invocation against a real MCP test server returns the
//!      expected result.
//!   2. Tool errors propagate as Lua errors with the server's error
//!      message attached.
//!   3. Cancellation: a tool invocation in flight when the calling
//!      coroutine is killed releases server-side resources cleanly.
//!
//! Plus the failure-mode coverage called out during the M9.3 design
//! review:
//!
//!   4. Two distinct error paths converge on the runtime's Failed
//!      outcome: MCP "tool errored" (isError: true) and JSON-RPC
//!      "method/error" (unknown tool name).
//!   5. Cancellation reaches the server: the fake's sentinel-file
//!      mechanism proves `notifications/cancelled` arrived.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use pmacs::async_runtime::{AsyncRuntime, JobId, JobOutcome, JobResult, SharedAsyncRuntime};
use pmacs::lua_bindings::SharedProcessSupervisor;
use pmacs::mcp::{
    McpClientState, McpEvent, McpEventKind, McpManager, McpRestartPolicy, McpServerId,
    McpServerSpec, SharedMcpManager,
};
use pmacs::process::ProcessSupervisor;

fn fake_mcp_path() -> String {
    env!("CARGO_BIN_EXE_pmacs_fake_mcp").to_owned()
}

fn make_test_triple() -> (
    SharedProcessSupervisor,
    SharedAsyncRuntime,
    SharedMcpManager,
) {
    let sup = Rc::new(RefCell::new(ProcessSupervisor::new()));
    let runtime: SharedAsyncRuntime = Rc::new(AsyncRuntime::with_pool_size(1));
    let mgr = Rc::new(RefCell::new(McpManager::new(sup.clone(), runtime.clone())));
    (sup, runtime, mgr)
}

fn fake_spec(label: &str) -> McpServerSpec {
    let mut spec = McpServerSpec::new(label, fake_mcp_path());
    spec.restart = McpRestartPolicy::Never;
    spec
}

fn pump_until<F: FnMut() -> bool>(
    sup: &SharedProcessSupervisor,
    runtime: &SharedAsyncRuntime,
    mgr: &SharedMcpManager,
    deadline: Duration,
    mut pred: F,
) {
    let stop = Instant::now() + deadline;
    while Instant::now() < stop {
        sup.borrow_mut().tick();
        mgr.borrow_mut().tick();
        runtime.tick();
        if pred() {
            return;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
}

fn drain_mcp_until<F: Fn(&[McpEvent]) -> bool>(
    sup: &SharedProcessSupervisor,
    runtime: &SharedAsyncRuntime,
    mgr: &SharedMcpManager,
    sid: McpServerId,
    deadline: Duration,
    pred: F,
) -> Vec<McpEvent> {
    let stop = Instant::now() + deadline;
    let mut all: Vec<McpEvent> = Vec::new();
    while Instant::now() < stop {
        sup.borrow_mut().tick();
        mgr.borrow_mut().tick();
        runtime.tick();
        let mut evs = mgr.borrow_mut().take_events(sid);
        all.append(&mut evs);
        if pred(&all) {
            return all;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    all
}

fn spawn_initialized(
    sup: &SharedProcessSupervisor,
    runtime: &SharedAsyncRuntime,
    mgr: &SharedMcpManager,
    spec: McpServerSpec,
) -> McpServerId {
    let sid = mgr.borrow_mut().spawn(spec).expect("spawn");
    drain_mcp_until(sup, runtime, mgr, sid, Duration::from_secs(5), |evs| {
        evs.iter()
            .any(|e| matches!(e.kind, McpEventKind::Initialized { .. }))
    });
    assert!(matches!(
        mgr.borrow().state(sid),
        Some(McpClientState::Initialized { .. })
    ));
    sid
}

fn await_job(
    sup: &SharedProcessSupervisor,
    runtime: &SharedAsyncRuntime,
    mgr: &SharedMcpManager,
    job_id: JobId,
    deadline: Duration,
) -> JobOutcome {
    pump_until(sup, runtime, mgr, deadline, || runtime.is_complete(job_id));
    runtime
        .take_result(job_id)
        .unwrap_or_else(|| panic!("job {job_id} did not settle within deadline"))
}

// ===========================================================================
// Bullet 1: round-trip
// ===========================================================================

/// Bullet 1: invoke a tool, get back its result. The fake's `echo`
/// tool produces `{ content: [{ type: "text", text: "<echo>: ..." }],
/// isError: false }`. The handle settles with that result table.
#[test]
fn m9_3_invoke_tool_round_trip() {
    let (sup, runtime, mgr) = make_test_triple();
    let sid = spawn_initialized(&sup, &runtime, &mgr, fake_spec("invoke"));
    let job = mgr
        .borrow_mut()
        .invoke_tool(sid, "echo", serde_json::json!({ "text": "hello mcp" }))
        .expect("invoke_tool");
    let outcome = await_job(&sup, &runtime, &mgr, job, Duration::from_secs(5));
    let result = match outcome {
        JobOutcome::Complete(JobResult::Json(v)) => v,
        other => panic!("expected Complete(Json(...)), got {other:?}"),
    };
    let text = result
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|p| p.get("text"))
        .and_then(|t| t.as_str())
        .expect("content[0].text");
    assert_eq!(text, "<echo>: hello mcp");
    assert_eq!(
        result.get("isError").and_then(serde_json::Value::as_bool),
        Some(false),
        "echo path must report isError: false"
    );
    let _ = mgr.borrow_mut().stop(sid);
}

// ===========================================================================
// Bullet 2 + design-review failure-mode coverage:
//   * MCP isError:true → Failed (with server message)
//   * JSON-RPC error response → Failed (with server message)
// ===========================================================================

/// Bullet 2 + design-review path A: a tool that returns
/// `isError: true` produces a Lua-visible Failed outcome with the
/// server's text-content as the message. The translation is a
/// deliberate API choice (see M9.3 audit) — callers don't have to
/// inspect `isError` themselves.
#[test]
fn m9_3_tool_iserror_translates_to_failed() {
    let (sup, runtime, mgr) = make_test_triple();
    let sid = spawn_initialized(&sup, &runtime, &mgr, fake_spec("iserror"));
    let job = mgr
        .borrow_mut()
        .invoke_tool(sid, "fail", serde_json::json!({}))
        .expect("invoke_tool");
    let outcome = await_job(&sup, &runtime, &mgr, job, Duration::from_secs(5));
    let msg = match outcome {
        JobOutcome::Failed(m) => m,
        other => panic!("expected Failed (isError -> Failed); got {other:?}"),
    };
    assert!(
        msg.contains("synthetic tool failure"),
        "Failed message must contain the server's tool error text; got {msg:?}"
    );
    let _ = mgr.borrow_mut().stop(sid);
}

/// Multipart-content tool error: text + non-text + text. Verifies
/// the message-extraction rules from the M9.3 audit:
/// - Order preserved (no reordering).
/// - Non-text parts → `[non-text content omitted]` placeholder.
/// - Text parts joined with newlines.
#[test]
fn m9_3_tool_iserror_extracts_multipart_content() {
    let (sup, runtime, mgr) = make_test_triple();
    let sid = spawn_initialized(&sup, &runtime, &mgr, fake_spec("multipart"));
    let job = mgr
        .borrow_mut()
        .invoke_tool(sid, "multipart_fail", serde_json::json!({}))
        .expect("invoke_tool");
    let outcome = await_job(&sup, &runtime, &mgr, job, Duration::from_secs(5));
    let msg = match outcome {
        JobOutcome::Failed(m) => m,
        other => panic!("expected Failed; got {other:?}"),
    };
    // Content order is `[text("Failed: "), image(...), text("see attached")]`.
    assert_eq!(
        msg, "Failed: \n[non-text content omitted]\nsee attached",
        "multipart extraction must preserve order and replace non-text with placeholder; got {msg:?}"
    );
    let _ = mgr.borrow_mut().stop(sid);
}

/// Design-review path B: a tool that the server doesn't know
/// produces a JSON-RPC error response. The standard Failed path
/// applies — the message includes the server's error code and text.
#[test]
fn m9_3_unknown_tool_produces_jsonrpc_error() {
    let (sup, runtime, mgr) = make_test_triple();
    let sid = spawn_initialized(&sup, &runtime, &mgr, fake_spec("unknown"));
    let job = mgr
        .borrow_mut()
        .invoke_tool(sid, "no_such_tool", serde_json::json!({}))
        .expect("invoke_tool");
    let outcome = await_job(&sup, &runtime, &mgr, job, Duration::from_secs(5));
    let msg = match outcome {
        JobOutcome::Failed(m) => m,
        other => panic!("expected Failed; got {other:?}"),
    };
    assert!(
        msg.contains("unknown tool: no_such_tool"),
        "JSON-RPC error message must contain the server's text; got {msg:?}"
    );
    assert!(
        msg.contains("-32602"),
        "Failed message should include the JSON-RPC error code; got {msg:?}"
    );
    let _ = mgr.borrow_mut().stop(sid);
}

// ===========================================================================
// Bullet 3: cancellation reaches the server
// ===========================================================================

/// Bullet 3: a tool invocation cancelled while in flight reaches the
/// server as `notifications/cancelled`. The fake's sentinel-file
/// mechanism (`PMACS_FAKE_MCP_CANCEL_DIR`) writes
/// `cancelled-<request_id>` on receipt, which the test polls for.
#[test]
fn m9_3_cancellation_reaches_server() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let cancel_dir = tmp.path().to_owned();
    let (sup, runtime, mgr) = make_test_triple();
    let mut spec = fake_spec("cancellation");
    spec.env = vec![
        ("PMACS_FAKE_MCP_MODE".into(), "slow_tools_call".into()),
        (
            "PMACS_FAKE_MCP_CANCEL_DIR".into(),
            cancel_dir.to_string_lossy().into_owned(),
        ),
    ];
    let sid = spawn_initialized(&sup, &runtime, &mgr, spec);

    let job = mgr
        .borrow_mut()
        .invoke_tool(sid, "echo", serde_json::json!({ "text": "slow" }))
        .expect("invoke_tool");

    // Cancel while the request is in flight (fake delays 250ms).
    runtime.cancel(job);

    // Pump until the handle settles AND the sentinel file appears.
    let stop = Instant::now() + Duration::from_secs(5);
    let mut sentinel_seen = false;
    while Instant::now() < stop {
        sup.borrow_mut().tick();
        mgr.borrow_mut().tick();
        runtime.tick();
        // Sentinel file: `cancelled-<request_id>`. We don't know
        // the request_id from the test — just look for any matching
        // file.
        if let Ok(entries) = std::fs::read_dir(&cancel_dir) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str()
                    && name.starts_with("cancelled-")
                {
                    sentinel_seen = true;
                    break;
                }
            }
        }
        if sentinel_seen && runtime.is_complete(job) {
            break;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    assert!(
        sentinel_seen,
        "fake should have written cancelled-<id> sentinel after notifications/cancelled"
    );
    assert!(
        runtime.is_complete(job),
        "cancelled handle should have settled"
    );
    let outcome = runtime.take_result(job).expect("settled");
    assert!(
        matches!(outcome, JobOutcome::Cancelled),
        "cancelled handle must settle as Cancelled; got {outcome:?}"
    );
    let _ = mgr.borrow_mut().stop(sid);
}

// ===========================================================================
// Lua surface
// ===========================================================================

/// `pmacs.mcp.invoke_tool(server, name, args):await()` resolves to
/// the response's `result` table on success, or raises a Lua error
/// (via the `tag = "failed"` async error path) on tool failure.
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "linear pump-coroutine-then-verify pattern; splitting fragments the test's narrative"
)]
fn m9_3_lua_invoke_tool_returns_awaitable_handle() {
    use pmacs::editor::EditorState;

    let mut state = EditorState::new();
    let fake = fake_mcp_path();

    state
        .lua_host
        .lua()
        .load(format!(
            "
            _G._mcp_test_server = pmacs.mcp.spawn({{
                label = 'lua-tool',
                command = '{fake}',
                restart = 'never',
            }})
            ",
        ))
        .exec()
        .expect("spawn via Lua");

    // Pump until Initialized.
    let stop = Instant::now() + Duration::from_secs(5);
    let mut initialized = false;
    while Instant::now() < stop && !initialized {
        state.tick_processes();
        state.tick_mcp();
        state.tick_async();
        let kinds: Vec<String> = state
            .lua_host
            .lua()
            .load(
                "
                local out = {}
                for _, row in ipairs(pmacs.mcp.list()) do
                    out[#out+1] = row.state.kind
                end
                return out
                ",
            )
            .eval()
            .expect("list");
        if kinds.iter().any(|k| k == "initialized") {
            initialized = true;
        }
        if !initialized {
            std::thread::sleep(Duration::from_millis(15));
        }
    }
    assert!(initialized, "server must reach Initialized");

    // Happy path: invoke `echo`, capture the text in a global.
    state
        .lua_host
        .lua()
        .load(
            "
            _G._mcp_tool_done = false
            _G._mcp_tool_text = nil
            _G._mcp_tool_failed = nil
            pmacs.async(function()
                local ok, result = pcall(function()
                    return pmacs.mcp.invoke_tool(_G._mcp_test_server, 'echo', { text = 'lua' }):await()
                end)
                if ok then
                    _G._mcp_tool_text = result.content[1].text
                else
                    _G._mcp_tool_failed = result
                end
                _G._mcp_tool_done = true
            end)
            ",
        )
        .exec()
        .expect("dispatch awaiting coroutine");

    let stop = Instant::now() + Duration::from_secs(5);
    let mut done = false;
    while Instant::now() < stop && !done {
        state.tick_processes();
        state.tick_mcp();
        state.tick_async();
        done = state
            .lua_host
            .lua()
            .load("return _G._mcp_tool_done")
            .eval::<bool>()
            .unwrap_or(false);
        if !done {
            std::thread::sleep(Duration::from_millis(15));
        }
    }
    assert!(done, "happy-path coroutine must complete");

    let text: Option<String> = state
        .lua_host
        .lua()
        .load("return _G._mcp_tool_text")
        .eval()
        .expect("read result text");
    assert_eq!(text.as_deref(), Some("<echo>: lua"));

    // Failure path: invoke `fail`, expect pcall to capture an error.
    state
        .lua_host
        .lua()
        .load(
            "
            _G._mcp_tool_done2 = false
            _G._mcp_tool_failed2 = nil
            pmacs.async(function()
                local ok, err = pcall(function()
                    return pmacs.mcp.invoke_tool(_G._mcp_test_server, 'fail', {}):await()
                end)
                _G._mcp_tool_done2 = true
                _G._mcp_tool_failed2 = (not ok) and err or nil
            end)
            ",
        )
        .exec()
        .expect("dispatch failure-path coroutine");

    let stop = Instant::now() + Duration::from_secs(5);
    let mut done2 = false;
    while Instant::now() < stop && !done2 {
        state.tick_processes();
        state.tick_mcp();
        state.tick_async();
        done2 = state
            .lua_host
            .lua()
            .load("return _G._mcp_tool_done2")
            .eval::<bool>()
            .unwrap_or(false);
        if !done2 {
            std::thread::sleep(Duration::from_millis(15));
        }
    }
    assert!(done2, "failure-path coroutine must complete");

    // Verify the raised error has the expected `tag = "failed"` /
    // `message` shape from the async runtime, and that the message
    // contains the server's text content.
    let message: String = state
        .lua_host
        .lua()
        .load(
            "
            local err = _G._mcp_tool_failed2
            if err == nil then return '<no error>' end
            if type(err) == 'table' then return tostring(err.message or '<no message>') end
            return tostring(err)
            ",
        )
        .eval()
        .expect("read error message");
    assert!(
        message.contains("synthetic tool failure"),
        "failure-path coroutine must observe the server's tool error text; got {message:?}"
    );

    let _ = state
        .lua_host
        .lua()
        .load("pmacs.mcp.stop(_G._mcp_test_server)")
        .exec();
}
