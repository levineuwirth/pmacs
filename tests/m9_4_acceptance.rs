// m9_4_acceptance.rs --- T M9.4 prompt resolution acceptance.

//! Acceptance tests for T M9.4 (`spec/pmacs-tasks.tex:3951`):
//!
//!   1. Prompt resolution returns the expected template-with-args.
//!   2. Required args missing produce a clear error.
//!   3. Prompts with no arguments are also supported.
//!
//! Plus the wire-shape verification called out during the M9.4
//! design review:
//!
//!   4. Empty-args call sends `arguments: {}` on the wire (not
//!      omitted, not `null`). The fake's prompt-record mechanism
//!      writes each request's params to disk; the test parses the
//!      JSON and asserts the shape.

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
// Bullet 1: round-trip with args
// ===========================================================================

/// Bullet 1: `get_prompt` with required args returns messages
/// referencing those args (the fake threads them through so the
/// test can verify they were transmitted intact).
#[test]
fn m9_4_get_prompt_round_trip_with_args() {
    let (sup, runtime, mgr) = make_test_triple();
    let sid = spawn_initialized(&sup, &runtime, &mgr, fake_spec("prompt-args"));
    let job = mgr
        .borrow_mut()
        .get_prompt(
            sid,
            "code_review",
            serde_json::json!({
                "language": "rust",
                "source": "fn main() {}",
            }),
        )
        .expect("get_prompt");
    let outcome = await_job(&sup, &runtime, &mgr, job, Duration::from_secs(5));
    let result = match outcome {
        JobOutcome::Complete(JobResult::Json(v)) => v,
        other => panic!("expected Complete(Json(...)), got {other:?}"),
    };
    let description = result
        .get("description")
        .and_then(|v| v.as_str())
        .expect("description field");
    assert!(
        description.contains("rust"),
        "description must contain the language arg; got {description:?}"
    );
    let text = result
        .get("messages")
        .and_then(|m| m.as_array())
        .and_then(|arr| arr.first())
        .and_then(|m| m.get("content"))
        .and_then(|c| c.get("text"))
        .and_then(|t| t.as_str())
        .expect("messages[0].content.text");
    assert!(
        text.contains("rust") && text.contains("fn main() {}"),
        "message text must contain both args; got {text:?}"
    );
    let _ = mgr.borrow_mut().stop(sid);
}

// ===========================================================================
// Bullet 2: missing required arg
// ===========================================================================

/// Bullet 2: `get_prompt` without a required arg produces a Lua
/// error via the standard JSON-RPC `Failed` path. The error
/// message includes the server's `-32602 missing required argument:
/// <name>` text.
#[test]
fn m9_4_missing_required_arg_produces_jsonrpc_error() {
    let (sup, runtime, mgr) = make_test_triple();
    let sid = spawn_initialized(&sup, &runtime, &mgr, fake_spec("prompt-missing"));
    // code_review requires {language, source}; omit source.
    let job = mgr
        .borrow_mut()
        .get_prompt(
            sid,
            "code_review",
            serde_json::json!({ "language": "rust" }),
        )
        .expect("get_prompt");
    let outcome = await_job(&sup, &runtime, &mgr, job, Duration::from_secs(5));
    let msg = match outcome {
        JobOutcome::Failed(m) => m,
        other => panic!("expected Failed; got {other:?}"),
    };
    assert!(
        msg.contains("missing required argument: source"),
        "Failed message must contain the server's missing-arg text; got {msg:?}"
    );
    assert!(
        msg.contains("-32602"),
        "Failed message should include the JSON-RPC error code; got {msg:?}"
    );
    let _ = mgr.borrow_mut().stop(sid);
}

// ===========================================================================
// Bullet 3: no-args prompt
// ===========================================================================

/// Bullet 3: prompts with no required arguments resolve cleanly
/// when called with an empty args object.
#[test]
fn m9_4_no_args_prompt_resolves() {
    let (sup, runtime, mgr) = make_test_triple();
    let sid = spawn_initialized(&sup, &runtime, &mgr, fake_spec("prompt-noargs"));
    let job = mgr
        .borrow_mut()
        .get_prompt(sid, "simple", serde_json::json!({}))
        .expect("get_prompt");
    let outcome = await_job(&sup, &runtime, &mgr, job, Duration::from_secs(5));
    let result = match outcome {
        JobOutcome::Complete(JobResult::Json(v)) => v,
        other => panic!("expected Complete(Json(...)), got {other:?}"),
    };
    let text = result
        .get("messages")
        .and_then(|m| m.as_array())
        .and_then(|arr| arr.first())
        .and_then(|m| m.get("content"))
        .and_then(|c| c.get("text"))
        .and_then(|t| t.as_str())
        .expect("messages[0].content.text");
    assert_eq!(text, "no-args prompt body");
    let _ = mgr.borrow_mut().stop(sid);
}

// ===========================================================================
// Wire-shape verification (design-review addition)
// ===========================================================================

/// The MCP spec requires `arguments` even when there are no
/// arguments — sending it as `{}` (empty object), not `null` and
/// not omitting the field. This test verifies the wire shape by
/// having the fake record each prompts/get's params to disk;
/// the test reads the recorded JSON and asserts `arguments` is
/// present and an empty object.
#[test]
fn m9_4_no_args_wire_shape_is_empty_object() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let record_dir = tmp.path().to_owned();
    let (sup, runtime, mgr) = make_test_triple();
    let mut spec = fake_spec("prompt-wire");
    spec.env = vec![(
        "PMACS_FAKE_MCP_PROMPT_RECORD_DIR".into(),
        record_dir.to_string_lossy().into_owned(),
    )];
    let sid = spawn_initialized(&sup, &runtime, &mgr, spec);
    // No-args call.
    let job = mgr
        .borrow_mut()
        .get_prompt(sid, "simple", serde_json::json!({}))
        .expect("get_prompt");
    let _ = await_job(&sup, &runtime, &mgr, job, Duration::from_secs(5));

    // Find the recorded params file.
    let mut entries: Vec<_> = std::fs::read_dir(&record_dir)
        .expect("read record dir")
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.starts_with("prompt-"))
        })
        .collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    assert_eq!(
        entries.len(),
        1,
        "fake should have recorded exactly one prompts/get; got {} files",
        entries.len()
    );
    let bytes = std::fs::read(entries[0].path()).expect("read recorded params");
    let params: serde_json::Value = serde_json::from_slice(&bytes).expect("parse recorded params");
    let arguments = params
        .get("arguments")
        .expect("params.arguments must be present (not omitted)");
    assert!(
        arguments.is_object(),
        "params.arguments must be an object, not null/array/string; got {arguments:?}"
    );
    assert_eq!(
        arguments.as_object().unwrap().len(),
        0,
        "params.arguments must be empty {{}}; got {arguments:?}"
    );
    let _ = mgr.borrow_mut().stop(sid);
}

// ===========================================================================
// Lua surface
// ===========================================================================

/// `pmacs.mcp.get_prompt(server, name, args):await()` resolves to
/// the response's `result` table. Three call patterns (no third
/// arg, nil third arg, explicit empty table) all produce the same
/// wire request and identical result.
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "linear pump-coroutine-then-verify pattern; splitting fragments the test's narrative"
)]
fn m9_4_lua_get_prompt_returns_awaitable_handle() {
    use pmacs::editor::EditorState;

    let mut state = EditorState::new_with_roots(&crate::iso::roots());
    let fake = fake_mcp_path();

    state
        .lua_host
        .lua()
        .load(format!(
            "
            _G._mcp_test_server = pmacs.mcp.spawn({{
                label = 'lua-prompt',
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

    // Three call forms, captured into separate globals. All three
    // should produce identical result tables.
    state
        .lua_host
        .lua()
        .load(
            "
            _G._mcp_prompt_done = 0
            _G._mcp_prompt_text_a = nil
            _G._mcp_prompt_text_b = nil
            _G._mcp_prompt_text_c = nil
            pmacs.async(function()
                local r = pmacs.mcp.get_prompt(_G._mcp_test_server, 'simple'):await()
                _G._mcp_prompt_text_a = r.messages[1].content.text
                _G._mcp_prompt_done = _G._mcp_prompt_done + 1
            end)
            pmacs.async(function()
                local r = pmacs.mcp.get_prompt(_G._mcp_test_server, 'simple', nil):await()
                _G._mcp_prompt_text_b = r.messages[1].content.text
                _G._mcp_prompt_done = _G._mcp_prompt_done + 1
            end)
            pmacs.async(function()
                local r = pmacs.mcp.get_prompt(_G._mcp_test_server, 'simple', {}):await()
                _G._mcp_prompt_text_c = r.messages[1].content.text
                _G._mcp_prompt_done = _G._mcp_prompt_done + 1
            end)
            ",
        )
        .exec()
        .expect("dispatch coroutines");

    let stop = Instant::now() + Duration::from_secs(5);
    let mut done = 0i64;
    while Instant::now() < stop && done < 3 {
        state.tick_processes();
        state.tick_mcp();
        state.tick_async();
        done = state
            .lua_host
            .lua()
            .load("return _G._mcp_prompt_done")
            .eval::<i64>()
            .unwrap_or(0);
        if done < 3 {
            std::thread::sleep(Duration::from_millis(15));
        }
    }
    assert_eq!(done, 3, "all three coroutines must complete");

    let texts: Vec<Option<String>> = state
        .lua_host
        .lua()
        .load("return { _G._mcp_prompt_text_a, _G._mcp_prompt_text_b, _G._mcp_prompt_text_c }")
        .eval()
        .expect("read texts");
    assert_eq!(
        texts,
        vec![
            Some("no-args prompt body".to_owned()),
            Some("no-args prompt body".to_owned()),
            Some("no-args prompt body".to_owned())
        ],
        "all three call forms must produce the same result"
    );

    // Failure path: missing required arg must raise a Lua error
    // visible through pcall.
    state
        .lua_host
        .lua()
        .load(
            "
            _G._mcp_prompt_done2 = false
            _G._mcp_prompt_failed2 = nil
            pmacs.async(function()
                local ok, err = pcall(function()
                    return pmacs.mcp.get_prompt(_G._mcp_test_server, 'code_review',
                                                { language = 'rust' }):await()
                end)
                _G._mcp_prompt_done2 = true
                _G._mcp_prompt_failed2 = (not ok) and err or nil
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
            .load("return _G._mcp_prompt_done2")
            .eval::<bool>()
            .unwrap_or(false);
        if !done2 {
            std::thread::sleep(Duration::from_millis(15));
        }
    }
    assert!(done2, "failure-path coroutine must complete");

    let message: String = state
        .lua_host
        .lua()
        .load(
            "
            local err = _G._mcp_prompt_failed2
            if err == nil then return '<no error>' end
            if type(err) == 'table' then return tostring(err.message or '<no message>') end
            return tostring(err)
            ",
        )
        .eval()
        .expect("read error message");
    assert!(
        message.contains("missing required argument: source"),
        "failure-path coroutine must observe the server's missing-arg text; got {message:?}"
    );

    let _ = state
        .lua_host
        .lua()
        .load("pmacs.mcp.stop(_G._mcp_test_server)")
        .exec();
}

// Isolated bootstrap storage roots (see the module docs): an
// integration test is compiled without `cfg(test)`, so a raw
// `EditorState::new()` would read the developer's real `init.lua` and
// write into their real data root.
#[path = "common/iso.rs"]
mod iso;
