// m9_1_acceptance.rs --- T M9.1 MCP worker variant acceptance.

//! Acceptance tests for T M9.1 (`spec/pmacs-tasks.tex:3891`):
//!
//!   1. MCP server supervised through the same supervisor as LSP.
//!   2. Initial `initialize` handshake completes; the server's
//!      declared capabilities are discoverable through the worker.
//!   3. Server crash and restart are handled identically to LSP.
//!
//! Plus the framing-claim cross-check called out during the M9.1
//! design review: the supervisor is protocol-agnostic, which is the
//! load-bearing property behind the "no new dispatch path" claim.
//! `lsp_and_mcp_can_coexist_on_one_supervisor` is the explicit test
//! for that — multiple MCPs is the easy case, LSP+MCP exercises the
//! protocol-agnostic property.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use pmacs::async_runtime::{AsyncRuntime, JobOutcome, JobResult, SharedAsyncRuntime};
use pmacs::lsp::{LspEventKind, LspManager, LspRestartPolicy, LspServerSpec};
use pmacs::lua_bindings::SharedProcessSupervisor;
use pmacs::mcp::{
    McpClientState, McpEvent, McpEventKind, McpManager, McpRestartPolicy, McpServerId,
    McpServerSpec, SharedMcpManager,
};
use pmacs::process::ProcessSupervisor;

fn fake_mcp_path() -> String {
    env!("CARGO_BIN_EXE_pmacs_fake_mcp").to_owned()
}

fn fake_lsp_path() -> String {
    env!("CARGO_BIN_EXE_pmacs_fake_lsp").to_owned()
}

fn make_test_triple() -> (
    SharedProcessSupervisor,
    SharedAsyncRuntime,
    SharedMcpManager,
) {
    let sup = Rc::new(RefCell::new(ProcessSupervisor::new()));
    // Pool of 1 is enough — MCP requests don't dispatch through the
    // pool (Pass-2 finding 1 wires response delivery via
    // register_external + complete_external_*), so the pool is
    // sized for tests that may want to mix MCP with another worker
    // dispatch in the same harness.
    let runtime: SharedAsyncRuntime = Rc::new(AsyncRuntime::with_pool_size(1));
    let mgr = Rc::new(RefCell::new(McpManager::new(sup.clone(), runtime.clone())));
    (sup, runtime, mgr)
}

/// Drain MCP events until `pred` is satisfied or the deadline lapses.
/// Mirrors `drain_lsp_until` from the M4 acceptance tests; also
/// drives the async runtime's tick so externally-settled jobs are
/// observable to callers that `take_result` on a `JobId`.
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

fn fake_spec(label: &str) -> McpServerSpec {
    let mut spec = McpServerSpec::new(label, fake_mcp_path());
    spec.restart = McpRestartPolicy::Never;
    spec
}

// ===========================================================================
// Acceptance bullet 2: initialize handshake + capability discovery
// ===========================================================================

/// The handshake completes and the server's declared capabilities
/// surface through the manager.
#[test]
fn m9_1_initialize_handshake_completes_and_capabilities_discoverable() {
    let (sup, runtime, mgr) = make_test_triple();
    let sid = mgr.borrow_mut().spawn(fake_spec("init")).expect("spawn");
    let evs = drain_mcp_until(&sup, &runtime, &mgr, sid, Duration::from_secs(5), |evs| {
        evs.iter()
            .any(|e| matches!(e.kind, McpEventKind::Initialized { .. }))
    });
    let caps = evs
        .iter()
        .find_map(|e| match &e.kind {
            McpEventKind::Initialized { capabilities } => Some(capabilities.clone()),
            _ => None,
        })
        .expect("must observe Initialized event");
    assert!(caps.is_object(), "capabilities should be a JSON object");
    // The fake server advertises resources, tools, and prompts.
    assert!(
        caps.get("resources").is_some(),
        "fake mcp must advertise resources, got {caps}"
    );
    assert!(
        caps.get("tools").is_some(),
        "fake mcp must advertise tools, got {caps}"
    );
    assert!(
        caps.get("prompts").is_some(),
        "fake mcp must advertise prompts, got {caps}"
    );
    // State surface mirrors event surface.
    let state = mgr.borrow().state(sid).cloned();
    assert!(matches!(state, Some(McpClientState::Initialized { .. })));
    // Capabilities also reachable via the worker-level getter (M9.1
    // acceptance: "discoverable through the worker").
    let mgr_caps = mgr
        .borrow()
        .capabilities(sid)
        .cloned()
        .expect("capabilities must be discoverable");
    assert_eq!(
        mgr_caps, caps,
        "manager capabilities must match the Initialized event's"
    );
    // Protocol version was echoed back. Pass-2 finding 3: pmacs
    // sends 2025-11-25 (the latest revision) and the fake server
    // echoes whatever the client sent.
    assert_eq!(mgr.borrow().protocol_version(sid), Some("2025-11-25"));
    let _ = mgr.borrow_mut().stop(sid);
}

#[test]
fn m9_1_server_info_surfaces_after_initialize() {
    let (sup, runtime, mgr) = make_test_triple();
    let sid = mgr.borrow_mut().spawn(fake_spec("info")).expect("spawn");
    drain_mcp_until(&sup, &runtime, &mgr, sid, Duration::from_secs(5), |evs| {
        evs.iter()
            .any(|e| matches!(e.kind, McpEventKind::Initialized { .. }))
    });
    let info = mgr
        .borrow()
        .server_info(sid)
        .cloned()
        .expect("serverInfo must be reported");
    assert_eq!(
        info.get("name").and_then(|v| v.as_str()),
        Some("pmacs-fake-mcp")
    );
    let _ = mgr.borrow_mut().stop(sid);
}

// ===========================================================================
// Acceptance bullet 3: crash + restart parity with LSP
// ===========================================================================

/// `OnCrash` policy respawns after the fake server's `exit 7`. The
/// observable signal is two `Started` events bracketing a `Crashed`
/// + `Restarting` — exactly the LSP test's shape.
#[test]
fn m9_1_server_crash_auto_restarts() {
    let (sup, runtime, mgr) = make_test_triple();
    mgr.borrow_mut()
        .set_restart_backoff(Duration::from_millis(50));
    let mut spec = fake_spec("crasher");
    spec.restart = McpRestartPolicy::OnCrash;
    spec.env = vec![("PMACS_FAKE_MCP_MODE".into(), "crash".into())];
    let sid = mgr.borrow_mut().spawn(spec).expect("spawn");
    let evs = drain_mcp_until(&sup, &runtime, &mgr, sid, Duration::from_secs(10), |evs| {
        evs.iter()
            .filter(|e| matches!(e.kind, McpEventKind::Started { .. }))
            .count()
            >= 2
    });
    let started_count = evs
        .iter()
        .filter(|e| matches!(e.kind, McpEventKind::Started { .. }))
        .count();
    assert!(
        started_count >= 2,
        "OnCrash policy should respawn after the fake MCP exits non-zero; saw Started count {started_count}"
    );
    assert!(
        evs.iter()
            .any(|e| matches!(e.kind, McpEventKind::Crashed { .. })),
        "must observe Crashed event after the fake MCP's exit 7"
    );
    assert!(
        evs.iter()
            .any(|e| matches!(e.kind, McpEventKind::Restarting { .. })),
        "must observe Restarting event when policy is OnCrash"
    );
    assert!(
        mgr.borrow().attempt(sid).unwrap_or(0) >= 2,
        "attempt count should be >= 2 after at least one restart"
    );
    let _ = mgr.borrow_mut().stop(sid);
}

/// After a crash + restart, the new generation re-runs `initialize`.
/// Two `Initialized` events prove the handshake replays, which is
/// the LSP-parity claim in concrete form.
#[test]
fn m9_1_restart_reruns_initialize_handshake() {
    let (sup, runtime, mgr) = make_test_triple();
    mgr.borrow_mut()
        .set_restart_backoff(Duration::from_millis(50));
    let mut spec = fake_spec("re-init");
    spec.restart = McpRestartPolicy::OnCrash;
    spec.env = vec![("PMACS_FAKE_MCP_MODE".into(), "crash".into())];
    let sid = mgr.borrow_mut().spawn(spec).expect("spawn");
    let evs = drain_mcp_until(&sup, &runtime, &mgr, sid, Duration::from_secs(10), |evs| {
        evs.iter()
            .filter(|e| matches!(e.kind, McpEventKind::Initialized { .. }))
            .count()
            >= 2
    });
    let init_count = evs
        .iter()
        .filter(|e| matches!(e.kind, McpEventKind::Initialized { .. }))
        .count();
    assert!(
        init_count >= 2,
        "must see Initialized for the original generation and the restart; saw {init_count}"
    );
    let _ = mgr.borrow_mut().stop(sid);
}

// ===========================================================================
// Acceptance bullet 1: same supervisor as LSP
// (multi-server coexistence; LSP+MCP cross-protocol)
// ===========================================================================

/// Two MCP servers spawned simultaneously through the same manager
/// reach `Initialized` independently. This is the easy case — both
/// peers speak the same wire format.
#[test]
fn m9_1_multiple_mcps_coexist_on_one_manager() {
    let (sup, runtime, mgr) = make_test_triple();
    let a = mgr.borrow_mut().spawn(fake_spec("a")).expect("spawn a");
    let b = mgr.borrow_mut().spawn(fake_spec("b")).expect("spawn b");
    drain_mcp_until(&sup, &runtime, &mgr, a, Duration::from_secs(5), |evs| {
        evs.iter()
            .any(|e| matches!(e.kind, McpEventKind::Initialized { .. }))
    });
    drain_mcp_until(&sup, &runtime, &mgr, b, Duration::from_secs(5), |evs| {
        evs.iter()
            .any(|e| matches!(e.kind, McpEventKind::Initialized { .. }))
    });
    assert!(matches!(
        mgr.borrow().state(a),
        Some(McpClientState::Initialized { .. })
    ));
    assert!(matches!(
        mgr.borrow().state(b),
        Some(McpClientState::Initialized { .. })
    ));
    let _ = mgr.borrow_mut().stop(a);
    let _ = mgr.borrow_mut().stop(b);
}

/// **The protocol-uniformity test.** An LSP server and an MCP server
/// share one [`ProcessSupervisor`]. Both reach `Initialized`. Neither
/// observes events from the other's process. If this test passes
/// without special wiring, the supervisor is genuinely
/// protocol-agnostic and the spec's "no new dispatch path" claim
/// holds. A failure would be a real M9.1 finding worth surfacing.
#[test]
fn m9_1_lsp_and_mcp_can_coexist_on_one_supervisor() {
    let sup = Rc::new(RefCell::new(ProcessSupervisor::new()));
    let runtime: SharedAsyncRuntime = Rc::new(AsyncRuntime::with_pool_size(1));
    let lsp_mgr = Rc::new(RefCell::new(LspManager::new(sup.clone(), runtime.clone())));
    let mcp_mgr = Rc::new(RefCell::new(McpManager::new(sup.clone(), runtime.clone())));

    // Spin up one LSP and one MCP through the same supervisor.
    let mut lsp_spec = LspServerSpec::new("co-lsp", "rust", fake_lsp_path());
    lsp_spec.restart = LspRestartPolicy::Never;
    let lsp_sid = lsp_mgr.borrow_mut().spawn(lsp_spec).expect("spawn lsp");
    let mcp_sid = mcp_mgr
        .borrow_mut()
        .spawn(fake_spec("co-mcp"))
        .expect("spawn mcp");

    // Drive both for up to 5s, each ticking through the shared
    // supervisor. The deadline is shared; we exit early once both
    // have initialized.
    let stop = Instant::now() + Duration::from_secs(5);
    let mut lsp_evs: Vec<LspEventKind> = Vec::new();
    let mut mcp_evs: Vec<McpEventKind> = Vec::new();
    while Instant::now() < stop {
        sup.borrow_mut().tick();
        lsp_mgr.borrow_mut().tick();
        mcp_mgr.borrow_mut().tick();
        let next_lsp: Vec<_> = lsp_mgr
            .borrow_mut()
            .take_events(lsp_sid)
            .into_iter()
            .map(|e| e.kind)
            .collect();
        let next_mcp: Vec<_> = mcp_mgr
            .borrow_mut()
            .take_events(mcp_sid)
            .into_iter()
            .map(|e| e.kind)
            .collect();
        lsp_evs.extend(next_lsp);
        mcp_evs.extend(next_mcp);
        let lsp_init = lsp_evs
            .iter()
            .any(|k| matches!(k, LspEventKind::Initialized { .. }));
        let mcp_init = mcp_evs
            .iter()
            .any(|k| matches!(k, McpEventKind::Initialized { .. }));
        if lsp_init && mcp_init {
            break;
        }
        std::thread::sleep(Duration::from_millis(15));
    }

    assert!(
        lsp_evs
            .iter()
            .any(|k| matches!(k, LspEventKind::Initialized { .. })),
        "LSP server must complete its handshake when sharing a supervisor with MCP; events: {lsp_evs:?}"
    );
    assert!(
        mcp_evs
            .iter()
            .any(|k| matches!(k, McpEventKind::Initialized { .. })),
        "MCP server must complete its handshake when sharing a supervisor with LSP; events: {mcp_evs:?}"
    );

    // Cross-leakage check: ProtocolError on either side would mean
    // bytes destined for one peer landed in the other's parser.
    assert!(
        !lsp_evs
            .iter()
            .any(|k| matches!(k, LspEventKind::ProtocolError { .. })),
        "LSP saw ProtocolError under coexistence; cross-leak suspected"
    );
    assert!(
        !mcp_evs
            .iter()
            .any(|k| matches!(k, McpEventKind::ProtocolError { .. })),
        "MCP saw ProtocolError under coexistence; cross-leak suspected"
    );

    let _ = lsp_mgr.borrow_mut().stop(lsp_sid);
    let _ = mcp_mgr.borrow_mut().stop(mcp_sid);
}

// ===========================================================================
// Auxiliary: send_request gating, forget, protocol violation
// ===========================================================================

/// `send_request` before the server is `Initialized` errors loudly
/// rather than silently dropping the bytes.
#[test]
fn m9_1_send_request_before_initialized_errors() {
    let (_sup, _runtime, mgr) = make_test_triple();
    let sid = mgr.borrow_mut().spawn(fake_spec("early")).expect("spawn");
    // Don't drain — server is in Starting/Initializing state.
    let res = mgr
        .borrow_mut()
        .send_request(sid, "ping", serde_json::Value::Null);
    assert!(
        res.is_err(),
        "send_request before Initialized must error; got {res:?}"
    );
    let _ = mgr.borrow_mut().stop(sid);
}

/// `ping` round-trips through the async runtime. Pass-2 finding 1:
/// `send_request` returns a `JobId`; the response settles the
/// runtime's pending entry rather than landing as a poll-style event
/// the caller has to scan for. `Response` events still fire for
/// observers, but the canonical settle path is the async runtime.
#[test]
fn m9_1_ping_request_settles_runtime_job() {
    let (sup, runtime, mgr) = make_test_triple();
    let sid = mgr.borrow_mut().spawn(fake_spec("ping")).expect("spawn");
    drain_mcp_until(&sup, &runtime, &mgr, sid, Duration::from_secs(5), |evs| {
        evs.iter()
            .any(|e| matches!(e.kind, McpEventKind::Initialized { .. }))
    });
    let job_id = mgr
        .borrow_mut()
        .send_request(sid, "ping", serde_json::Value::Null)
        .expect("send_request");
    // Drive the manager + runtime until the job settles. We can't
    // use drain_mcp_until's predicate over events because the
    // canonical settle is on the runtime side, not in the
    // McpManager event queue.
    let stop = Instant::now() + Duration::from_secs(5);
    while Instant::now() < stop && !runtime.is_complete(job_id) {
        sup.borrow_mut().tick();
        mgr.borrow_mut().tick();
        runtime.tick();
        std::thread::sleep(Duration::from_millis(15));
    }
    assert!(
        runtime.is_complete(job_id),
        "ping job must settle within deadline"
    );
    let outcome = runtime.take_result(job_id).expect("take_result");
    let value = match outcome {
        JobOutcome::Complete(JobResult::Json(v)) => v,
        other => panic!("expected Complete(Json(...)), got {other:?}"),
    };
    assert!(
        value.is_object(),
        "fake mcp returns `result: {{}}` for ping; got {value}"
    );
    let _ = mgr.borrow_mut().stop(sid);
}

/// A garbage line on the wire surfaces as a `ProtocolError` event.
/// Mirrors the LSP protocol-violation acceptance test.
#[test]
fn m9_1_protocol_violation_surfaces_as_structured_error() {
    let (sup, runtime, mgr) = make_test_triple();
    let mut spec = fake_spec("garbager");
    spec.env = vec![("PMACS_FAKE_MCP_MODE".into(), "garbage".into())];
    let sid = mgr.borrow_mut().spawn(spec).expect("spawn");
    let evs = drain_mcp_until(&sup, &runtime, &mgr, sid, Duration::from_secs(5), |evs| {
        evs.iter().any(|e| {
            matches!(
                e.kind,
                McpEventKind::ProtocolError { .. }
                    | McpEventKind::Crashed { .. }
                    | McpEventKind::Stopped
            )
        })
    });
    assert!(
        evs.iter()
            .any(|e| matches!(&e.kind, McpEventKind::ProtocolError { .. })),
        "non-JSON line should surface as a ProtocolError; got events: {:?}",
        evs.iter().map(|e| &e.kind).collect::<Vec<_>>()
    );
    let _ = mgr.borrow_mut().stop(sid);
}

/// `forget` removes a terminal-state server from the registry; a
/// non-terminal server can't be forgotten.
#[test]
fn m9_1_forget_only_terminal_servers() {
    let (sup, runtime, mgr) = make_test_triple();
    let sid = mgr.borrow_mut().spawn(fake_spec("forget")).expect("spawn");
    drain_mcp_until(&sup, &runtime, &mgr, sid, Duration::from_secs(5), |evs| {
        evs.iter()
            .any(|e| matches!(e.kind, McpEventKind::Initialized { .. }))
    });
    // Forgetting an Initialized server must error.
    assert!(
        mgr.borrow_mut().forget(sid).is_err(),
        "forget should reject a non-terminal server"
    );
    // After stop, the server transitions through ShuttingDown to
    // Stopped; once Stopped, forget succeeds.
    let _ = mgr.borrow_mut().stop(sid);
    drain_mcp_until(&sup, &runtime, &mgr, sid, Duration::from_secs(5), |_| {
        matches!(
            mgr.borrow().state(sid),
            Some(McpClientState::Stopped { .. } | McpClientState::Crashed { .. })
        )
    });
    assert!(
        mgr.borrow_mut().forget(sid).is_ok(),
        "forget should succeed once the server is in a terminal state"
    );
    assert!(
        mgr.borrow().state(sid).is_none(),
        "forgotten server's state must be None"
    );
}

// ===========================================================================
// Pass-2 findings 2 / 3 / 5: lifecycle correctness
// ===========================================================================

/// Pass-2 finding 2: a JSON-RPC error response to `initialize` must
/// **not** transition the client to `Initialized`. The fake server's
/// `init_error` mode replies with an error code; pmacs should
/// surface a `ProtocolError` event and terminate the process rather
/// than masquerading as a healthy server.
#[test]
fn m9_1_initialize_error_does_not_transition_to_initialized() {
    let (sup, runtime, mgr) = make_test_triple();
    let mut spec = fake_spec("init_error");
    spec.env = vec![("PMACS_FAKE_MCP_MODE".into(), "init_error".into())];
    let sid = mgr.borrow_mut().spawn(spec).expect("spawn");
    let evs = drain_mcp_until(&sup, &runtime, &mgr, sid, Duration::from_secs(5), |evs| {
        evs.iter()
            .any(|e| matches!(e.kind, McpEventKind::ProtocolError { .. }))
    });
    assert!(
        evs.iter()
            .any(|e| matches!(&e.kind, McpEventKind::ProtocolError { message } if message.contains("refused initialize"))),
        "must observe ProtocolError citing the initialize refusal; got {:?}",
        evs.iter().map(|e| &e.kind).collect::<Vec<_>>()
    );
    assert!(
        !evs.iter()
            .any(|e| matches!(e.kind, McpEventKind::Initialized { .. })),
        "Initialized must not fire when initialize returned error"
    );
    // The state never transitions to Initialized; capabilities()
    // returns None.
    assert!(
        !matches!(
            mgr.borrow().state(sid),
            Some(McpClientState::Initialized { .. })
        ),
        "client state must not be Initialized after error response"
    );
    assert!(mgr.borrow().capabilities(sid).is_none());
    let _ = mgr.borrow_mut().stop(sid);
}

/// Pass-2 finding 3: an unsupported `protocolVersion` in the
/// `initialize` response must terminate the connection rather than
/// silently accepting it. The fake's `bad_version` mode reports
/// `1999-01-01`, which is not in pmacs's supported set.
#[test]
fn m9_1_unsupported_protocol_version_is_rejected() {
    let (sup, runtime, mgr) = make_test_triple();
    let mut spec = fake_spec("bad_version");
    spec.env = vec![("PMACS_FAKE_MCP_MODE".into(), "bad_version".into())];
    let sid = mgr.borrow_mut().spawn(spec).expect("spawn");
    let evs = drain_mcp_until(&sup, &runtime, &mgr, sid, Duration::from_secs(5), |evs| {
        evs.iter()
            .any(|e| matches!(e.kind, McpEventKind::ProtocolError { .. }))
    });
    assert!(
        evs.iter().any(|e| matches!(
            &e.kind,
            McpEventKind::ProtocolError { message }
                if message.contains("unsupported protocolVersion")
        )),
        "must observe ProtocolError citing the unsupported protocolVersion; got {:?}",
        evs.iter().map(|e| &e.kind).collect::<Vec<_>>()
    );
    assert!(
        !matches!(
            mgr.borrow().state(sid),
            Some(McpClientState::Initialized { .. })
        ),
        "client must not reach Initialized for an unsupported protocolVersion"
    );
    let _ = mgr.borrow_mut().stop(sid);
}

/// Pass-3 finding 1: MCP stdio shutdown is stdin-EOF + live SIGTERM /
/// SIGKILL fallback. The client must
/// **not** send protocol-level `shutdown` requests or `exit`
/// notifications. The fake's `crash_on_protocol_shutdown` mode
/// exits with code 99 if it ever sees those methods on the wire,
/// so a Stopped event (clean exit code 0) proves pmacs took the
/// EOF path.
#[test]
fn m9_1_stop_uses_stdio_eof_not_protocol_shutdown() {
    let (sup, runtime, mgr) = make_test_triple();
    let mut spec = fake_spec("polite-stop");
    spec.env = vec![(
        "PMACS_FAKE_MCP_MODE".into(),
        "crash_on_protocol_shutdown".into(),
    )];
    let sid = mgr.borrow_mut().spawn(spec).expect("spawn");
    drain_mcp_until(&sup, &runtime, &mgr, sid, Duration::from_secs(5), |evs| {
        evs.iter()
            .any(|e| matches!(e.kind, McpEventKind::Initialized { .. }))
    });
    let _ = mgr.borrow_mut().stop(sid);
    let evs = drain_mcp_until(&sup, &runtime, &mgr, sid, Duration::from_secs(5), |_| {
        matches!(
            mgr.borrow().state(sid),
            Some(McpClientState::Stopped { .. } | McpClientState::Crashed { .. })
        )
    });
    let final_state = mgr.borrow().state(sid).cloned();
    assert!(
        matches!(final_state, Some(McpClientState::Stopped { .. })),
        "stop() must transition to Stopped via stdin EOF (clean exit), not Crashed; got {:?}; events: {:?}",
        final_state,
        evs.iter().map(|e| &e.kind).collect::<Vec<_>>()
    );
    assert!(
        !evs.iter()
            .any(|e| matches!(e.kind, McpEventKind::Crashed { .. })),
        "no Crashed event — server must exit cleanly on stdin EOF; got {:?}",
        evs.iter().map(|e| &e.kind).collect::<Vec<_>>()
    );
}

/// Pass-4 finding 1: EOF is the compliant first shutdown signal, but
/// a server that ignores EOF must still be stopped while the manager
/// is live. This fixture sleeps forever after stdin closes; the
/// shortened grace window lets the manager send SIGTERM and observe a
/// terminal Stopped state.
#[test]
fn m9_1_stop_escalates_when_server_ignores_stdin_eof() {
    let (sup, runtime, mgr) = make_test_triple();
    mgr.borrow_mut()
        .set_shutdown_grace(Duration::from_millis(25));
    let mut spec = fake_spec("ignore-eof");
    spec.env = vec![("PMACS_FAKE_MCP_MODE".into(), "ignore_eof_sleep".into())];
    let sid = mgr.borrow_mut().spawn(spec).expect("spawn");
    drain_mcp_until(&sup, &runtime, &mgr, sid, Duration::from_secs(5), |evs| {
        evs.iter()
            .any(|e| matches!(e.kind, McpEventKind::Initialized { .. }))
    });
    let _ = mgr.borrow_mut().stop(sid);
    let evs = drain_mcp_until(&sup, &runtime, &mgr, sid, Duration::from_secs(5), |_| {
        matches!(
            mgr.borrow().state(sid),
            Some(McpClientState::Stopped { .. } | McpClientState::Crashed { .. })
        )
    });
    let final_state = mgr.borrow().state(sid).cloned();
    assert!(
        matches!(final_state, Some(McpClientState::Stopped { .. })),
        "stop() must keep escalating after stdin EOF until the process exits; got {:?}; events: {:?}",
        final_state,
        evs.iter().map(|e| &e.kind).collect::<Vec<_>>()
    );
}

/// Pass-3 finding 2: an `initialize` response missing the required
/// `capabilities` field is a protocol violation. pmacs must surface
/// `ProtocolError` and refuse to transition to `Initialized`,
/// rather than defaulting capabilities to `Null` and proceeding.
#[test]
fn m9_1_missing_capabilities_in_initialize_is_rejected() {
    let (sup, runtime, mgr) = make_test_triple();
    let mut spec = fake_spec("missing_caps");
    spec.env = vec![("PMACS_FAKE_MCP_MODE".into(), "missing_caps".into())];
    let sid = mgr.borrow_mut().spawn(spec).expect("spawn");
    let evs = drain_mcp_until(&sup, &runtime, &mgr, sid, Duration::from_secs(5), |evs| {
        evs.iter()
            .any(|e| matches!(e.kind, McpEventKind::ProtocolError { .. }))
    });
    assert!(
        evs.iter().any(|e| matches!(
            &e.kind,
            McpEventKind::ProtocolError { message }
                if message.contains("missing required capabilities")
        )),
        "must observe ProtocolError citing missing capabilities; got {:?}",
        evs.iter().map(|e| &e.kind).collect::<Vec<_>>()
    );
    assert!(
        !evs.iter()
            .any(|e| matches!(e.kind, McpEventKind::Initialized { .. })),
        "Initialized must not fire when capabilities is missing"
    );
    assert!(
        !matches!(
            mgr.borrow().state(sid),
            Some(McpClientState::Initialized { .. })
        ),
        "client state must not be Initialized after a malformed result"
    );
    assert!(mgr.borrow().capabilities(sid).is_none());
    let _ = mgr.borrow_mut().stop(sid);
}

/// Pass-2 finding 5: `OnCrash` policy must not restart on a clean
/// exit (code 0). The fake's `clean_exit_after_init` mode replies
/// to initialize then exits 0 without us asking. The manager
/// should observe `Stopped`, not `Crashed`/`Restarting`.
#[test]
fn m9_1_oncrash_policy_does_not_restart_on_clean_exit() {
    let (sup, runtime, mgr) = make_test_triple();
    mgr.borrow_mut()
        .set_restart_backoff(Duration::from_millis(50));
    let mut spec = fake_spec("clean_exit");
    spec.restart = McpRestartPolicy::OnCrash;
    spec.env = vec![("PMACS_FAKE_MCP_MODE".into(), "clean_exit_after_init".into())];
    let sid = mgr.borrow_mut().spawn(spec).expect("spawn");
    // Drain for a while to give a buggy implementation room to
    // restart; the assertion is about what didn't happen.
    let evs = drain_mcp_until(&sup, &runtime, &mgr, sid, Duration::from_secs(2), |evs| {
        evs.iter().any(|e| matches!(e.kind, McpEventKind::Stopped))
    });
    assert!(
        evs.iter().any(|e| matches!(e.kind, McpEventKind::Stopped)),
        "clean exit + OnCrash must surface as Stopped, not Crashed/Restarting; got {:?}",
        evs.iter().map(|e| &e.kind).collect::<Vec<_>>()
    );
    assert!(
        !evs.iter()
            .any(|e| matches!(e.kind, McpEventKind::Restarting { .. })),
        "OnCrash must not respawn on clean exit; got {:?}",
        evs.iter().map(|e| &e.kind).collect::<Vec<_>>()
    );
    let started_count = evs
        .iter()
        .filter(|e| matches!(e.kind, McpEventKind::Started { .. }))
        .count();
    assert_eq!(
        started_count, 1,
        "exactly one Started event for a single clean-exit generation; got {started_count}"
    );
}

// ===========================================================================
// Lua surface
// ===========================================================================

/// `pmacs.mcp.spawn` / `_tick` / `events_take` / `capabilities` agree
/// with the Rust-level view. Smoke test of the boundary marshalling.
#[test]
fn m9_1_lua_surface_drives_mcp_lifecycle() {
    use pmacs::editor::EditorState;

    let mut state = EditorState::new_with_roots(&crate::iso::roots());
    let fake = fake_mcp_path();
    let lua = state.lua_host.lua();
    let sid_raw: u64 = lua
        .load(format!(
            "
            local id = pmacs.mcp.spawn({{
                label = 'lua-fake',
                command = '{fake}',
                restart = 'never',
            }})
            return id:raw()
            ",
        ))
        .eval()
        .expect("spawn via Lua");
    assert!(sid_raw > 0);

    // Pump until Initialized.
    let stop = Instant::now() + Duration::from_secs(5);
    let mut initialized = false;
    while Instant::now() < stop && !initialized {
        state.tick_processes();
        state.tick_mcp();
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
    assert!(initialized, "lua-spawned MCP must reach Initialized");

    // capabilities() should return a non-nil table.
    let has_caps: bool = state
        .lua_host
        .lua()
        .load(
            "
            local rows = pmacs.mcp.list()
            assert(#rows == 1, 'expected one server')
            local caps = pmacs.mcp.capabilities(rows[1].id)
            return caps ~= nil and type(caps) == 'table' and caps.resources ~= nil
            ",
        )
        .eval()
        .expect("capabilities");
    assert!(
        has_caps,
        "pmacs.mcp.capabilities should return the server's caps table"
    );

    // Stop and let it drain.
    let _ = state
        .lua_host
        .lua()
        .load(
            "
        for _, row in ipairs(pmacs.mcp.list()) do
            pmacs.mcp.stop(row.id)
        end
        ",
        )
        .exec();
}

/// Pass-2 finding 1 acceptance: `pmacs.mcp.send_request` returns a
/// Handle, and awaiting that handle inside `pmacs.async(...)`
/// resumes with the response's `result` table — same dispatch shape
/// as `pmacs.workers.compute_sum`, `pmacs.fs.read_dir`, etc. This is
/// the shape M9.2/M9.3 will build on; verifying it here means the
/// follow-up tasks don't need to introduce a separate async layer.
#[test]
fn m9_1_lua_send_request_returns_awaitable_handle() {
    use pmacs::editor::EditorState;

    let mut state = EditorState::new_with_roots(&crate::iso::roots());
    let fake = fake_mcp_path();
    let lua = state.lua_host.lua();

    // Spawn the server and stash its id in a global the next
    // `pmacs.async` block reads.
    lua.load(format!(
        "
        _G._mcp_test_server = pmacs.mcp.spawn({{
            label = 'lua-await',
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

    // Spawn the awaiting coroutine. The result lands in a global
    // when the handle settles, which we observe from Rust below.
    state
        .lua_host
        .lua()
        .load(
            "
            _G._mcp_test_done = false
            _G._mcp_test_result = nil
            pmacs.async(function()
                local result = pmacs.mcp.send_request(_G._mcp_test_server, 'ping', {}):await()
                _G._mcp_test_result = result
                _G._mcp_test_done = true
            end)
            ",
        )
        .exec()
        .expect("dispatch awaiting coroutine");

    // Pump until the coroutine completes.
    let stop = Instant::now() + Duration::from_secs(5);
    let mut done = false;
    while Instant::now() < stop && !done {
        state.tick_processes();
        state.tick_mcp();
        state.tick_async();
        done = state
            .lua_host
            .lua()
            .load("return _G._mcp_test_done")
            .eval::<bool>()
            .unwrap_or(false);
        if !done {
            std::thread::sleep(Duration::from_millis(15));
        }
    }
    assert!(
        done,
        "awaiting coroutine must complete (Handle.await() didn't resume)"
    );

    // The fake's `ping` reply is `result: {}`. The Lua side received
    // an empty table, which we verify by checking the type.
    let result_type: String = state
        .lua_host
        .lua()
        .load("return type(_G._mcp_test_result)")
        .eval()
        .expect("read result type");
    assert_eq!(
        result_type, "table",
        "ping response should marshal to a Lua table; got {result_type}"
    );

    // Stop the server.
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
