// mcp.rs --- T M9.1 Model Context Protocol client core.

//! Model Context Protocol client core. Spec §sec:m9-ai. JSON-RPC 2.0
//! over newline-delimited JSON on a child process's stdin/stdout
//! (the MCP-stdio transport), riding on the same
//! [`crate::process::ProcessSupervisor`] (M4.4) the LSP layer uses.
//!
//! # Topology
//!
//! ```text
//!   main thread          supervisor (M4.4)         MCP server
//!   -----------          -------------------       -------------
//!   McpManager      ---> spawn + write_stdin --->  child stdin
//!     ^                                            child stdout
//!     +-- line parse <--- Stdout(Vec<u8>) <------- reader thread
//!     +-- line parse <--- Stderr(Vec<u8>) <------- reader thread
//! ```
//!
//! The supervisor's reader thread already chunks pipe output off the
//! main thread; the MCP layer just buffers those chunks per server
//! and splits newline-delimited JSON bodies out of them in
//! [`McpManager::tick`]. Parsing is incremental: a body split across
//! two `Stdout` events is reassembled at the buffer boundary without
//! re-allocating.
//!
//! # Protocol uniformity
//!
//! M9 makes good on the protocol-uniformity claim from
//! [spec §sec:concurrency]: MCP rides on the same dispatch path as
//! LSP (supervisor → bytes → parser → state machine → events). The
//! only LSP/MCP difference is the framer — `Content-Length` headers
//! for LSP, `\n`-delimited bodies for MCP. The dispatch machinery
//! itself is unchanged. See M9.1 audit for the explicit
//! "no new dispatch path" walkthrough.
//!
//! # Lifecycle
//!
//! ```text
//!   Starting --(supervisor: Started)--> Initializing
//!     --(initialize response)--> Initialized { capabilities }
//!     --(stdin EOF / SIGTERM / SIGKILL + exit)--> Stopped
//!     --(unexpected exit / signal)--> Crashed
//!     --(restart policy)--> Initializing  (new pid)
//! ```
//!
//! Mirrors LSP exactly: every transition emits an [`McpEvent`]
//! visible through [`McpManager::take_events`], crashes restart
//! automatically per [`McpRestartPolicy`], and a restart re-runs
//! `initialize` from scratch (MCP servers, like LSP servers, do not
//! survive their process).
//!
//! # Concurrency
//!
//! A single [`McpManager`] lives on the main thread inside the
//! editor's `Rc<RefCell<...>>`, sharing the supervisor with
//! `pmacs.lsp.*` and `pmacs.process.*`. All inbound bytes arrive on
//! supervisor reader threads (one per pipe); all outbound writes go
//! through [`crate::process::ProcessSupervisor::write_stdin`]
//! synchronously from the main thread. Per-server traffic is small
//! (handshake + occasional resource/tool requests), so the
//! synchronous write is acceptable for v0.1.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use nix::sys::signal::Signal;
use serde_json::{Map, Value, json};

use crate::async_runtime::{JobId, JobKind, SharedAsyncRuntime};
use crate::process::{
    ProcessEvent, ProcessEventKind, ProcessId, ProcessMode, ProcessSpec, RestartPolicy,
};
use crate::worker::CancellationToken;

// ---------------------------------------------------------------------------
// Protocol version negotiation
// ---------------------------------------------------------------------------

/// MCP protocol version pmacs prefers when initializing. The MCP
/// lifecycle spec mandates that the client sends the latest version
/// it supports; pmacs targets the current revision. See
/// <https://modelcontextprotocol.io/specification/latest/basic/lifecycle>.
pub const PREFERRED_PROTOCOL_VERSION: &str = "2025-11-25";

/// Versions pmacs accepts in an `initialize` response. If the server
/// echoes back any of these, the handshake completes; any other value
/// is treated as a protocol violation and the server is terminated.
/// We accept the immediately-prior revision as a back-compat
/// concession (real-world servers lag the latest spec). A future
/// pmacs may add or drop entries here as the MCP version landscape
/// evolves.
pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2024-11-05"];

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

/// Stable identifier for one managed MCP server. Allocated in
/// monotonic order from a process-wide counter; an MCP id and an LSP
/// id share no namespace so the two are always distinguishable.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct McpServerId(u64);

impl McpServerId {
    /// Mint a fresh id.
    #[must_use]
    pub fn next() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        Self(COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    /// Raw counter value. Used for Lua boundary marshalling and
    /// debug formatting.
    #[must_use]
    pub fn raw(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for McpServerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "McpServerId({})", self.0)
    }
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Spec for one MCP server.
///
/// MCP's `initialize` request carries protocol version, client
/// capabilities, and client info. The pmacs implementation supplies
/// sensible defaults for all three; this spec carries only what the
/// process supervisor needs (command + env + cwd) plus the restart
/// policy. M9.2+ may extend this surface as concrete consumers
/// (resource fetching, tool invocation) reveal what fields users
/// actually need to override.
#[derive(Clone, Debug)]
pub struct McpServerSpec {
    /// Human-readable label. Surfaced in events and the
    /// `pmacs.mcp.list` output.
    pub label: String,
    /// Program to execute. PATH-resolved unless absolute.
    pub command: String,
    /// Argument vector.
    pub args: Vec<String>,
    /// Working directory for the spawn. `None` inherits from the
    /// editor process.
    pub cwd: Option<PathBuf>,
    /// Environment overrides for the child process.
    pub env: Vec<(String, String)>,
    /// Restart policy for the server. Mirrors
    /// [`crate::process::RestartPolicy`] and parallels the LSP
    /// equivalent.
    pub restart: McpRestartPolicy,
}

impl McpServerSpec {
    /// Construct a spec with the bare-minimum fields.
    #[must_use]
    pub fn new(label: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            command: command.into(),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
            restart: McpRestartPolicy::OnCrash,
        }
    }

    fn to_process_spec(&self) -> ProcessSpec {
        let mut p = ProcessSpec::new(format!("mcp:{}", self.label), &self.command);
        p.args.clone_from(&self.args);
        p.cwd.clone_from(&self.cwd);
        p.env.clone_from(&self.env);
        p.mode = ProcessMode::Pipes;
        // We handle restart at the MCP layer (because we need to
        // re-run `initialize` after each restart); the supervisor
        // never restarts behind our back.
        p.restart = RestartPolicy::Never;
        p
    }
}

/// What to do when the MCP server process terminates unexpectedly.
/// Parallels [`crate::lsp::LspRestartPolicy`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpRestartPolicy {
    /// Never restart.
    Never,
    /// Restart on signal or non-zero exit.
    OnCrash,
    /// Restart on any termination (clean or otherwise).
    Always,
}

// ---------------------------------------------------------------------------
// Lifecycle state
// ---------------------------------------------------------------------------

/// MCP client state machine. One instance per managed server.
/// Parallels [`crate::lsp::LspClientState`].
#[derive(Clone, Debug)]
pub enum McpClientState {
    /// Process spawning; supervisor hasn't reported `Started` yet.
    Starting,
    /// Process is alive; `initialize` request is in flight.
    Initializing {
        /// JSON-RPC id of the in-flight `initialize` request.
        init_request_id: u64,
        /// When the server process started.
        started: Instant,
    },
    /// `initialize` has been answered; `notifications/initialized`
    /// has been sent. The server is ready for arbitrary requests.
    Initialized {
        /// `capabilities` returned by the server's `initialize`
        /// result. The full JSON is preserved; M9.2+ projects the
        /// bits each consumer needs.
        capabilities: Value,
        /// `serverInfo` from the initialize result, if reported.
        server_info: Option<Value>,
        /// `protocolVersion` as echoed back by the server. Stored
        /// verbatim so a future M9.x can validate against pmacs's
        /// supported set.
        protocol_version: Option<String>,
        /// When the server reached this state.
        initialized_at: Instant,
    },
    /// User-initiated shutdown is in progress: the client has closed
    /// the child's stdin and is waiting for the process to exit.
    /// Pass-3 finding 1: MCP stdio has no protocol-level shutdown
    /// message — closing stdin is the canonical EOF signal, with
    /// SIGTERM after one grace window and SIGKILL after another. See
    /// <https://modelcontextprotocol.io/specification/latest/basic/lifecycle>.
    ShuttingDown,
    /// Process exited cleanly (either after stdin EOF, after
    /// user-stop escalation, or on its own).
    Stopped {
        /// When the process exited.
        ended: Instant,
    },
    /// Process died unexpectedly.
    Crashed {
        /// Why it died.
        reason: String,
        /// When the supervisor reported the termination.
        ended: Instant,
    },
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// One MCP-layer event. Visible to the editor via
/// [`McpManager::take_events`] and to Lua via `pmacs.mcp.events_take`.
/// Parallels [`crate::lsp::LspEvent`].
#[derive(Clone, Debug)]
pub struct McpEvent {
    /// Server the event belongs to.
    pub server: McpServerId,
    /// What happened.
    pub kind: McpEventKind,
    /// When the event was generated (monotonic).
    pub at: Instant,
}

/// Discriminator over MCP-layer event kinds. Parallels
/// [`crate::lsp::LspEventKind`].
#[derive(Clone, Debug)]
pub enum McpEventKind {
    /// Process spawned and is running. The `initialize` request was
    /// just sent.
    Started {
        /// OS pid of the new generation.
        pid: u32,
    },
    /// `initialize` response was received and
    /// `notifications/initialized` was sent.
    Initialized {
        /// Server `capabilities` from the response.
        capabilities: Value,
    },
    /// Server-originated notification. MCP servers send these for
    /// `notifications/resources/updated`, etc.
    Notification {
        /// JSON-RPC method name.
        method: String,
        /// Params payload (already JSON-decoded).
        params: Value,
    },
    /// Server-originated request. MCP servers may issue
    /// `roots/list` etc. against the client; pmacs surfaces these so
    /// a consumer can `send_response`.
    Request {
        /// Inbound id; the client will respond with this id.
        id: Value,
        /// JSON-RPC method.
        method: String,
        /// Params payload.
        params: Value,
    },
    /// Response to a client-initiated request was received.
    Response {
        /// Id of the original request (echoed back).
        id: u64,
        /// `result` field; `Null` if absent.
        result: Value,
        /// `error` field, if any (in which case `result` is `Null`).
        error: Option<McpError>,
        /// Method of the original request, copied here for
        /// convenience so consumers don't have to track ids.
        method: String,
    },
    /// User shutdown started by closing stdin; the next terminal
    /// process event observed will be the actual exit.
    ShuttingDown,
    /// Server process exited cleanly. Final state for this generation.
    Stopped,
    /// Server process died (signal or non-zero exit outside
    /// user-initiated shutdown).
    Crashed {
        /// Display-formatted reason.
        reason: String,
    },
    /// The manager is about to restart this server.
    Restarting {
        /// Cumulative spawn attempt for this server (1 = first
        /// spawn, 2 = first restart, ...).
        attempt: u32,
    },
    /// Bytes from the server's stderr. Servers commonly write log
    /// chatter here.
    Stderr(Vec<u8>),
    /// A protocol violation was observed and recovered from.
    ProtocolError {
        /// Display-formatted description of what went wrong.
        message: String,
    },
}

/// JSON-RPC error object (`{code, message, data?}`). Parallels
/// [`crate::lsp::LspError`]; the JSON-RPC layer is the same.
#[derive(Clone, Debug)]
pub struct McpError {
    /// Numeric error code.
    pub code: i64,
    /// Human-readable message.
    pub message: String,
    /// Optional `data` field, free-form JSON.
    pub data: Option<Value>,
}

impl McpError {
    fn from_value(v: &Value) -> Self {
        let code = v.get("code").and_then(Value::as_i64).unwrap_or(0);
        let message = v
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let data = v.get("data").cloned();
        Self {
            code,
            message,
            data,
        }
    }
}

// ---------------------------------------------------------------------------
// Frame codec
// ---------------------------------------------------------------------------

/// Incremental newline-delimited frame parser for JSON-RPC over MCP
/// stdio. Holds a single `Vec<u8>` buffer that grows with appended
/// bytes; complete bodies (terminated by `\n`) are extracted off the
/// front and returned to the caller.
///
/// The MCP-stdio spec uses `\n` (LF) as the body terminator. Embedded
/// `\n` inside a JSON string would corrupt framing — but JSON
/// serializers escape control characters by default, so a
/// well-formed implementation never produces them. The parser does
/// not attempt to recover from a misframed peer; an unrecoverable
/// frame error is the caller's signal to terminate the server.
#[derive(Default)]
pub struct NdjsonParser {
    buf: Vec<u8>,
}

impl NdjsonParser {
    /// New empty parser.
    #[must_use]
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Append `chunk` to the internal buffer.
    pub fn extend(&mut self, chunk: &[u8]) {
        self.buf.extend_from_slice(chunk);
    }

    /// Pop one complete body off the front of the buffer if a
    /// `\n` terminator is present, returning the body bytes (without
    /// the terminator). Returns `None` if no complete body is
    /// buffered yet. Empty bodies are skipped silently.
    pub fn next_frame(&mut self) -> Option<Vec<u8>> {
        loop {
            let nl = self.buf.iter().position(|&b| b == b'\n')?;
            let mut body = self.buf[..nl].to_vec();
            self.buf.drain(..=nl);
            // Trim a trailing `\r` so peers using CRLF line endings
            // (rare but legal in JSON) don't break parsing.
            if body.last() == Some(&b'\r') {
                body.pop();
            }
            if body.is_empty() {
                continue;
            }
            return Some(body);
        }
    }
}

/// Encode a JSON-RPC body as a newline-terminated MCP-stdio frame.
/// The newline is required; peers split on `\n`.
#[must_use]
pub fn encode_frame(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 1);
    out.extend_from_slice(body);
    out.push(b'\n');
    out
}

// ---------------------------------------------------------------------------
// McpClient
// ---------------------------------------------------------------------------

/// One Lua-visible awaiter bound to an in-flight request. Each
/// awaiter carries its own [`CancellationToken`] (from the runtime's
/// `register_external`) so per-awaiter cancellation is independent:
/// cancelling one handle settles that handle without disturbing the
/// other awaiters or aborting the in-flight wire request.
///
/// Pass-2 (M9.2) finding 2: prior to this fix, sibling awaiters
/// attached via coalescing only stored a [`JobId`], so:
/// (a) cancelling a sibling didn't wake it until the primary
/// completed, and (b) cancelling the primary cancelled every
/// sibling regardless of whether they still wanted the result.
/// Both violated the async-Handle contract.
#[derive(Clone, Debug)]
struct Awaiter {
    job_id: JobId,
    token: CancellationToken,
}

/// In-flight request bound to one or more Lua-visible async-runtime
/// jobs. The JSON-RPC response, when it lands, settles every
/// non-cancelled awaiter via
/// [`SharedAsyncRuntime::complete_external_ok`] (or `_failed` if the
/// server returned an error).
///
/// `cache_key` is `Some` for requests dispatched via the resource
/// cache (T M9.2 `read_resource`); on settlement, the cache entry
/// transitions `InFlight` → `Cached` if the cache still references
/// this request (`Ok` outcome only) or `InFlight` → Absent on
/// failure/cancellation/invalidation. `awaiters` is the live set of
/// callers waiting for this response; per-awaiter cancellation
/// removes entries one at a time without disturbing the in-flight
/// request, while the entry itself disappears once `awaiters` is
/// empty.
#[derive(Clone, Debug)]
struct PendingExternal {
    method: String,
    /// Cache slot to settle alongside this request. `None` for
    /// non-cached requests (regular `send_request`).
    cache_key: Option<(McpServerId, String)>,
    /// Awaiters waiting for the response. The first entry is the
    /// caller who originally dispatched the request; later entries
    /// are siblings attached via M3.5 coalescing. All settle
    /// equivalently when the response lands; any subset can cancel
    /// independently. Invariant: never empty when this entry exists
    /// in `pending_external` (the entry is removed when the last
    /// awaiter cancels).
    awaiters: Vec<Awaiter>,
}

/// Outcome handed to [`McpManager::settle_in_flight_with`] — the
/// canonical exit point for any in-flight external request. The rule
/// the helper enforces: **`InFlight` → `Cached` only on `Ok` for a
/// request whose cache slot still references this `request_id`;**
/// every other terminal state results in `InFlight` → Absent. T M9.2.
#[derive(Clone, Debug)]
enum SettleOutcome {
    Ok(Value),
    Failed(String),
    Cancelled,
}

/// One drained pending entry ready to feed into
/// [`McpManager::settle_in_flight_with`]: the cache key, the
/// request id, and the awaiters (one or more) that were waiting
/// for the response. T M9.2.
type DrainedPending = (Option<(McpServerId, String)>, u64, Vec<Awaiter>);

/// State machine for one entry in the per-(server, uri) resource
/// cache. T M9.2.
///
/// - `InFlight { request_id }` — a `resources/read` request is on
///   the wire under that JSON-RPC id. The cache entry is the
///   coordination point for M3.5 coalescing: subsequent
///   `read_resource` calls for the same key attach to the existing
///   request rather than dispatching afresh.
/// - `Cached { result }` — the most recent successful result.
///   `read_resource` returns it via the runtime; `invalidate_resource`
///   drops the entry.
///
/// "Absent" is encoded as the entry being missing from the cache map.
#[derive(Clone, Debug)]
enum ResourceCacheState {
    InFlight {
        /// JSON-RPC id of the in-flight request. The settlement
        /// helper compares this against the response's id so a
        /// stale response that arrives after `invalidate_resource`
        /// does not clobber a fresh in-flight or Cached entry.
        request_id: u64,
    },
    Cached {
        result: Value,
    },
}

/// Discriminator for [`McpManager::handle_response`]'s lookup phase.
/// `Internal` matches the in-manager bookkeeping for `initialize`;
/// `External` matches a Lua-visible
/// `pmacs.mcp.send_request`; `Unknown` is the response-for-no-request
/// protocol error.
enum PendingKind {
    Internal(String),
    External(PendingExternal),
    Unknown,
}

/// Next shutdown escalation to perform for a child that has not
/// exited after stdin EOF.
#[derive(Copy, Clone, Debug)]
enum ShutdownEscalation {
    SigtermAt(Instant),
    SigkillAt(Instant),
}

/// One managed MCP server: process + JSON-RPC framing + state
/// machine. Owned by [`McpManager`]; the supervisor handles process
/// I/O underneath.
pub struct McpClient {
    spec: McpServerSpec,
    state: McpClientState,
    process: Option<ProcessId>,
    stdout: NdjsonParser,
    /// JSON-RPC request id counter. Monotonically increasing.
    next_request_id: u64,
    /// Internal-request bookkeeping. Holds the method name for
    /// requests the manager itself originates (`initialize`);
    /// responses surface as state-machine transitions rather than as
    /// Lua-visible events. Keeping the internal table separate from
    /// `pending_external` means there is no risk of settling an
    /// async-runtime job for an internal request that has no job id.
    pending_internal: HashMap<u64, String>,
    /// External-request bookkeeping. Holds the `(method, job_id,
    /// token)` triple for requests dispatched through
    /// `pmacs.mcp.send_request`. The response settles the
    /// corresponding async-runtime job.
    pending_external: HashMap<u64, PendingExternal>,
    /// Cumulative spawn attempts (1 = first spawn, 2 = first
    /// restart, ...).
    attempt: u32,
    /// When the manager should attempt the next restart, if any.
    next_restart_at: Option<Instant>,
    /// Next live shutdown escalation, if the child has not exited
    /// after stdin EOF.
    /// Pass-3 finding 1.
    shutdown_escalation: Option<ShutdownEscalation>,
    /// Request ids that we've sent `notifications/cancelled` for and
    /// no longer have awaiters listening on. T M9.3: when a late
    /// response for one of these arrives (cancel/response race), we
    /// silently drop it rather than emitting a "response for unknown
    /// request id" `ProtocolError`. Bounded by user-initiated
    /// cancellations; cleared on terminal/restart.
    cancelled_rids: std::collections::HashSet<u64>,
}

impl McpClient {
    fn new(spec: McpServerSpec) -> Self {
        Self {
            spec,
            state: McpClientState::Starting,
            process: None,
            stdout: NdjsonParser::new(),
            next_request_id: 1,
            pending_internal: HashMap::new(),
            pending_external: HashMap::new(),
            attempt: 0,
            next_restart_at: None,
            shutdown_escalation: None,
            cancelled_rids: std::collections::HashSet::new(),
        }
    }

    /// Current spec.
    #[must_use]
    pub fn spec(&self) -> &McpServerSpec {
        &self.spec
    }

    /// Current lifecycle state.
    #[must_use]
    pub fn state(&self) -> &McpClientState {
        &self.state
    }

    /// Process id of the running generation, if any.
    #[must_use]
    pub fn process(&self) -> Option<ProcessId> {
        self.process
    }

    /// Server capabilities once initialized; `None` otherwise.
    #[must_use]
    pub fn capabilities(&self) -> Option<&Value> {
        match &self.state {
            McpClientState::Initialized { capabilities, .. } => Some(capabilities),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// McpManager
// ---------------------------------------------------------------------------

/// Per-editor registry of [`McpClient`]s. Owns no threads of its own;
/// ticks via [`Self::tick`] from the run loop. Holds a handle to the
/// async runtime so per-request response delivery settles
/// Lua-visible job ids without occupying a worker thread.
pub struct McpManager {
    supervisor: crate::lua_bindings::SharedProcessSupervisor,
    runtime: SharedAsyncRuntime,
    clients: HashMap<McpServerId, McpClient>,
    /// Reverse lookup: which server is which process.
    process_to_server: HashMap<ProcessId, McpServerId>,
    /// Per-server pending event buffer.
    pending: HashMap<McpServerId, Vec<McpEvent>>,
    /// Constant restart back-off.
    restart_backoff: Duration,
    /// How long to wait between shutdown escalation stages:
    /// stdin EOF -> SIGTERM -> SIGKILL. Pass-3 finding 1. Default
    /// 1s; tunable for tests.
    shutdown_grace: Duration,
    /// Per-(server, uri) resource cache. T M9.2. Cache hits return
    /// the stored result via the async runtime; concurrent
    /// `read_resource` calls coalesce onto a single in-flight
    /// request via the `InFlight` state.
    ///
    /// Cache is per-process, in-memory; cleared on editor restart
    /// and on `pmacs.packages.reload`. Persistent caching is not in
    /// scope for v0.1.
    resource_cache: HashMap<(McpServerId, String), ResourceCacheState>,
    /// Notification dispatcher (T M9.5). Method names that have at
    /// least one Lua-side handler registered via
    /// `pmacs.mcp.on_notification`. Notifications matching a
    /// subscribed method are queued in `notification_queue` for the
    /// Lua-side tick to drain and dispatch. Notifications for
    /// non-subscribed methods are not queued (they still surface
    /// via `events_take` for callers using the raw event stream).
    ///
    /// M9.5–M9.7 share this dispatcher: M9.5 hooks
    /// `notifications/resources/updated`, M9.6 hooks
    /// `notifications/tools/list_changed`, M9.7 will hook
    /// `notifications/prompts/list_changed`. One walk over events
    /// per tick, dispatched per-method.
    notification_subscriptions: std::collections::HashSet<String>,
    /// Per-method queue of `(server, params)` pairs awaiting Lua-
    /// side dispatch. Drained by `_drain_notifications`. T M9.5.
    notification_queue: HashMap<String, Vec<(McpServerId, Value)>>,
}

impl McpManager {
    /// Construct a fresh manager wired to `supervisor` and the
    /// editor's `runtime`. The runtime is the bridge between the
    /// supervisor's reader-thread response delivery and Lua-side
    /// `Handle:await()` resumption.
    #[must_use]
    pub fn new(
        supervisor: crate::lua_bindings::SharedProcessSupervisor,
        runtime: SharedAsyncRuntime,
    ) -> Self {
        Self {
            supervisor,
            runtime,
            clients: HashMap::new(),
            process_to_server: HashMap::new(),
            pending: HashMap::new(),
            restart_backoff: Duration::from_millis(500),
            shutdown_grace: Duration::from_secs(1),
            resource_cache: HashMap::new(),
            notification_subscriptions: std::collections::HashSet::new(),
            notification_queue: HashMap::new(),
        }
    }

    /// T M9.5: register interest in a JSON-RPC notification method.
    /// Once subscribed, every `notifications/<method>` arriving from
    /// any server is queued for Lua-side dispatch via
    /// [`Self::drain_notifications`]. Idempotent.
    pub fn subscribe_notification(&mut self, method: impl Into<String>) {
        self.notification_subscriptions.insert(method.into());
    }

    /// T M9.5: drop interest in a method. Pending queued
    /// notifications for the method are also dropped.
    pub fn unsubscribe_notification(&mut self, method: &str) {
        self.notification_subscriptions.remove(method);
        self.notification_queue.remove(method);
    }

    /// T M9.5: drain all queued notifications. Returns a map
    /// `method → [(server, params), ...]` and clears the queues.
    /// Called by the Lua-side tick to dispatch handlers; one walk
    /// per tick, all subscribed methods drained.
    pub fn drain_notifications(&mut self) -> HashMap<String, Vec<(McpServerId, Value)>> {
        std::mem::take(&mut self.notification_queue)
    }

    /// Override the restart back-off. Test helper.
    pub fn set_restart_backoff(&mut self, d: Duration) {
        self.restart_backoff = d;
    }

    /// Override the stdin-close-to-SIGTERM grace window. Test helper.
    pub fn set_shutdown_grace(&mut self, d: Duration) {
        self.shutdown_grace = d;
    }

    /// Spawn a server. Synchronous; the spawn itself is reported via
    /// the next tick's `Started` event (or `Crashed` if the supervisor
    /// refused the exec).
    pub fn spawn(&mut self, spec: McpServerSpec) -> Result<McpServerId, String> {
        let id = McpServerId::next();
        let mut client = McpClient::new(spec);
        self.start_generation(id, &mut client)?;
        self.clients.insert(id, client);
        Ok(id)
    }

    fn start_generation(&mut self, id: McpServerId, client: &mut McpClient) -> Result<(), String> {
        client.attempt += 1;
        client.next_restart_at = None;
        client.shutdown_escalation = None;
        client.stdout = NdjsonParser::new();
        client.pending_internal.clear();
        // T M9.3: drop the cancelled-rid set; the new generation has
        // fresh request-id numbering, so any outstanding rids from
        // the previous generation can never collide with a new
        // response.
        client.cancelled_rids.clear();
        // Drop any in-flight external requests onto the floor as
        // cancelled. The new generation has fresh request-id numbering;
        // no response on the new channel could match the old job ids
        // anyway, so the await()s never wake without this cancel.
        // T M9.2: route through `settle_in_flight_with` so awaiters
        // and cache state transitions are uniform.
        let cancelled: Vec<DrainedPending> = client
            .pending_external
            .drain()
            .map(|(rid, p)| (p.cache_key, rid, p.awaiters))
            .collect();
        for (cache_key, rid, awaiters) in cancelled {
            self.settle_in_flight_with(cache_key, rid, &awaiters, SettleOutcome::Cancelled);
        }
        // Pass-2 (M9.2) finding 1: clear cached entries for this
        // server before the new generation starts. Cached results
        // belong to the previous generation; the post-restart
        // server may serve different content for the same URI.
        self.drop_resource_cache_for(id);
        client.state = McpClientState::Starting;
        let proc_spec = client.spec.to_process_spec();
        let pid = self.supervisor.borrow_mut().spawn(proc_spec)?;
        client.process = Some(pid);
        self.process_to_server.insert(pid, id);
        Ok(())
    }

    /// Drop every cache entry whose key starts with `sid`. Called on
    /// terminal-state transitions, on `forget`, and on restart so a
    /// stale `read_resource(old_sid, uri)` against a server that has
    /// stopped, been forgotten, or restarted does not return cache
    /// data. Pass-2 (M9.2) finding 1.
    fn drop_resource_cache_for(&mut self, sid: McpServerId) {
        self.resource_cache.retain(|(s, _), _| *s != sid);
    }

    /// Initiate shutdown. Pass-3 finding 1: MCP stdio has no
    /// protocol-level shutdown message. The compliant path is to
    /// close the child's stdin (signaling EOF), wait for the child
    /// to exit on its own, then escalate to SIGTERM after a grace
    /// window and SIGKILL after a second grace window.
    /// See <https://modelcontextprotocol.io/specification/latest/basic/lifecycle>.
    ///
    /// Idempotent: a second call against an already-stopping or
    /// already-terminal server is a no-op.
    pub fn stop(&mut self, id: McpServerId) -> Result<(), String> {
        let Some(client) = self.clients.get_mut(&id) else {
            return Err(format!("unknown server: {id}"));
        };
        // Disable restart so a clean exit doesn't trigger a respawn.
        client.spec.restart = McpRestartPolicy::Never;
        client.next_restart_at = None;
        // Already terminal or already stopping — nothing to do.
        if matches!(
            client.state,
            McpClientState::Stopped { .. }
                | McpClientState::Crashed { .. }
                | McpClientState::ShuttingDown
        ) {
            return Ok(());
        }
        // Close stdin so the child sees EOF on its next read.
        // Servers that follow the MCP lifecycle spec exit on EOF;
        // anything still alive after `shutdown_grace` gets SIGTERM,
        // then SIGKILL after another grace window.
        if let Some(pid) = client.process {
            let _ = self.supervisor.borrow_mut().close_stdin(pid);
        }
        client.state = McpClientState::ShuttingDown;
        client.shutdown_escalation = Some(ShutdownEscalation::SigtermAt(
            Instant::now() + self.shutdown_grace,
        ));
        Ok(())
    }

    /// Send a JSON-RPC request to `id`. Returns the async-runtime
    /// [`JobId`] the response will settle. The Lua surface wraps that
    /// id in a `pmacs.workers` Handle; package code awaits the handle
    /// and receives the response's `result` value as a Lua table (or
    /// the structured `error` raised through `Handle:await()`'s
    /// failure path). Mirrors the dispatch shape of the M3-built
    /// handlers (`pmacs.workers.compute_sum`, `pmacs.fs.read_dir`,
    /// etc.) — the server's response settles a runtime job rather
    /// than a per-protocol poll-style queue. T M9.1 / Pass-2 finding 1.
    pub fn send_request(
        &mut self,
        id: McpServerId,
        method: impl Into<String>,
        params: Value,
    ) -> Result<JobId, String> {
        let method = method.into();
        let client = self
            .clients
            .get_mut(&id)
            .ok_or_else(|| format!("unknown server: {id}"))?;
        if !matches!(client.state, McpClientState::Initialized { .. }) {
            return Err(format!(
                "server {id} is not ready for requests (state: {})",
                state_label(&client.state)
            ));
        }
        let req_id = next_request_id(client);
        let body = make_request(req_id, &method, params);
        let (job_id, token) = self.runtime.register_external(JobKind::McpRequest, None);
        client.pending_external.insert(
            req_id,
            PendingExternal {
                method,
                cache_key: None,
                awaiters: vec![Awaiter { job_id, token }],
            },
        );
        if let Err(e) = send_frame_to(&self.supervisor, client, &body) {
            // Roll back the registration: the request never made it
            // onto the wire, so no response will ever arrive. Settle
            // every awaiter (just the one here) as failed so the
            // handle wakes with an error rather than blocking forever.
            if let Some(p) = client.pending_external.remove(&req_id) {
                let msg = format!("send_request: {e}");
                for a in p.awaiters {
                    self.runtime.complete_external_failed(a.job_id, msg.clone());
                }
            }
            return Err(e);
        }
        Ok(job_id)
    }

    /// Read a resource via `resources/read`. T M9.2.
    ///
    /// Returns the async-runtime [`JobId`] the response will settle.
    /// Three paths:
    ///
    /// 1. **Cache hit (`Cached`)** — registers a fresh job and
    ///    immediately settles it with the stored result. The
    ///    awaiter wakes one runtime tick later. The dispatch shape
    ///    is the contract; cached values still arrive through a
    ///    Handle.
    /// 2. **In-flight (`InFlight`)** — registers a fresh job and
    ///    attaches it to the primary's `sibling_awaiters`. M3.5
    ///    coalescing: 10 concurrent `read_resource` calls produce
    ///    1 wire request and 10 awaiters that all wake with the
    ///    same response.
    /// 3. **Cache miss (`Absent`)** — dispatches a fresh
    ///    `resources/read`, marks the cache entry `InFlight`,
    ///    returns the primary's job id.
    ///
    /// Errors during the wire write roll back the cache state so
    /// subsequent reads can re-dispatch.
    pub fn read_resource(
        &mut self,
        sid: McpServerId,
        uri: impl Into<String>,
    ) -> Result<JobId, String> {
        let uri = uri.into();

        // Pass-2 (M9.2) finding 1: validate the server exists and is
        // initialized BEFORE consulting the cache. A cached value is
        // only valid in the context of a running, initialized
        // server; a stale handle that survived `stop`/`forget` (or
        // worse, points at an `McpServerId` that was reused for an
        // unrelated server) must not return cache data.
        let client = self
            .clients
            .get_mut(&sid)
            .ok_or_else(|| format!("unknown server: {sid}"))?;
        if !matches!(client.state, McpClientState::Initialized { .. }) {
            return Err(format!(
                "server {sid} is not ready for requests (state: {})",
                state_label(&client.state)
            ));
        }

        let key = (sid, uri.clone());

        // (1) Cache hit.
        if let Some(ResourceCacheState::Cached { result }) = self.resource_cache.get(&key).cloned()
        {
            let (job_id, _token) = self.runtime.register_external(JobKind::McpRequest, None);
            self.runtime.complete_external_ok(job_id, result);
            return Ok(job_id);
        }

        // (2) In-flight: attach a fresh awaiter (with its own
        // CancellationToken) to the existing entry. Pass-2 (M9.2)
        // finding 2: per-awaiter tokens let siblings cancel
        // independently.
        if let Some(ResourceCacheState::InFlight { request_id }) = self.resource_cache.get(&key) {
            let in_flight_rid = *request_id;
            let (job_id, token) = self.runtime.register_external(JobKind::McpRequest, None);
            if let Some(p) = client.pending_external.get_mut(&in_flight_rid) {
                p.awaiters.push(Awaiter { job_id, token });
                return Ok(job_id);
            }
            // Race: cache claims InFlight but the entry's bookkeeping
            // is gone. Drop the stale cache entry and fall through
            // to the cache-miss path so we re-dispatch.
            self.resource_cache.remove(&key);
            self.runtime.complete_external_cancelled(job_id);
        }

        // (3) Cache miss: dispatch.
        let req_id = next_request_id(client);
        let body = make_request(req_id, "resources/read", json!({ "uri": uri }));
        let (job_id, token) = self.runtime.register_external(JobKind::McpRequest, None);
        client.pending_external.insert(
            req_id,
            PendingExternal {
                method: "resources/read".to_owned(),
                cache_key: Some(key.clone()),
                awaiters: vec![Awaiter { job_id, token }],
            },
        );
        if let Err(e) = send_frame_to(&self.supervisor, client, &body) {
            if let Some(p) = client.pending_external.remove(&req_id) {
                let msg = format!("read_resource: {e}");
                for a in p.awaiters {
                    self.runtime.complete_external_failed(a.job_id, msg.clone());
                }
            }
            return Err(e);
        }
        // Successful write — install the InFlight marker. Any
        // subsequent read_resource for the same key attaches as a
        // sibling rather than dispatching afresh.
        self.resource_cache
            .insert(key, ResourceCacheState::InFlight { request_id: req_id });
        Ok(job_id)
    }

    /// Invalidate the cache entry for `(sid, uri)`. T M9.2.
    ///
    /// Per the M9.2 design (option `i`): invalidation while in-flight
    /// settles existing awaiters with the arriving result, but does
    /// **not** re-cache. A subsequent `read_resource` after
    /// invalidation re-dispatches, ensuring the post-invalidation
    /// reader observes a fresh fetch (per spec: "subsequent reads
    /// refetch"). The mechanism: `settle_in_flight_with` checks
    /// whether the cache entry still references the response's
    /// request id; after invalidation it does not, so the result
    /// flows to awaiters but the cache stays Absent.
    pub fn invalidate_resource(&mut self, sid: McpServerId, uri: impl Into<String>) {
        let key = (sid, uri.into());
        self.resource_cache.remove(&key);
    }

    /// Invoke a tool via `tools/call`. T M9.3.
    ///
    /// Returns the async-runtime [`JobId`] the response will settle.
    /// Each `invoke_tool` is a fresh wire request — no caching, no
    /// coalescing (tool calls are distinct, may have side effects).
    ///
    /// Two error paths converge on the runtime's Failed outcome:
    /// 1. **JSON-RPC error response** — the server returns
    ///    `{"error":{...}}`. Standard `settle_in_flight_with`
    ///    Failed path applies.
    /// 2. **MCP "tool errored" success response** — the server
    ///    returns `{"result":{"isError": true, "content": [...]}}`.
    ///    The response handler's per-method translator inspects
    ///    the `isError` flag and converts it to `SettleOutcome::
    ///    Failed` with the extracted text. The translation is a
    ///    deliberate API choice, not implementation detail: the
    ///    contract is `invoke_tool` raises Lua errors on tool
    ///    failure; callers needing structured access to the raw
    ///    `{isError, content}` table use
    ///    `pmacs.mcp.send_request("tools/call", ...)` to bypass
    ///    the translator.
    pub fn invoke_tool(
        &mut self,
        sid: McpServerId,
        name: impl Into<String>,
        arguments: Value,
    ) -> Result<JobId, String> {
        // `arguments` is consumed below: the request body moves it
        // into the JSON-RPC params map. clippy's
        // needless_pass_by_value heuristic doesn't see that through
        // the build steps.
        let name = name.into();
        let client = self
            .clients
            .get_mut(&sid)
            .ok_or_else(|| format!("unknown server: {sid}"))?;
        if !matches!(client.state, McpClientState::Initialized { .. }) {
            return Err(format!(
                "server {sid} is not ready for requests (state: {})",
                state_label(&client.state)
            ));
        }
        let req_id = next_request_id(client);
        // Build params explicitly so `arguments` is moved (rather
        // than referenced by `json!`); avoids a needless-pass-by-
        // value clippy complaint and matches `send_request`'s shape.
        let mut params_map = Map::new();
        params_map.insert("name".into(), Value::String(name));
        params_map.insert("arguments".into(), arguments);
        let body = make_request(req_id, "tools/call", Value::Object(params_map));
        let (job_id, token) = self.runtime.register_external(JobKind::McpRequest, None);
        client.pending_external.insert(
            req_id,
            PendingExternal {
                method: "tools/call".to_owned(),
                cache_key: None,
                awaiters: vec![Awaiter { job_id, token }],
            },
        );
        if let Err(e) = send_frame_to(&self.supervisor, client, &body) {
            if let Some(p) = client.pending_external.remove(&req_id) {
                let msg = format!("invoke_tool: {e}");
                for a in p.awaiters {
                    self.runtime.complete_external_failed(a.job_id, msg.clone());
                }
            }
            return Err(e);
        }
        Ok(job_id)
    }

    /// Resolve a prompt template via `prompts/get`. T M9.4.
    ///
    /// Returns the async-runtime [`JobId`] the response will settle.
    /// Each `get_prompt` is a fresh wire request — no caching, no
    /// coalescing.
    ///
    /// Unlike `tools/call`, `prompts/get` has **no `isError`-style
    /// semantic-failure path**. The MCP spec defines exactly two
    /// outcomes: success (returns `messages`) or JSON-RPC error
    /// (typically `-32602` for missing required args, unknown
    /// prompt, etc.). The standard `Failed` path covers the latter
    /// without translation.
    ///
    /// `arguments` is sent as the `arguments` field of the request
    /// params verbatim (after Lua-to-JSON marshaling). The MCP spec
    /// requires the field even when there are no arguments; the
    /// Lua boundary at `_get_prompt_raw` translates `None`/`Nil` to
    /// an empty object `{}` rather than omitting the field.
    pub fn get_prompt(
        &mut self,
        sid: McpServerId,
        name: impl Into<String>,
        arguments: Value,
    ) -> Result<JobId, String> {
        // `arguments` is consumed below via the params Map::insert.
        let name = name.into();
        let client = self
            .clients
            .get_mut(&sid)
            .ok_or_else(|| format!("unknown server: {sid}"))?;
        if !matches!(client.state, McpClientState::Initialized { .. }) {
            return Err(format!(
                "server {sid} is not ready for requests (state: {})",
                state_label(&client.state)
            ));
        }
        let req_id = next_request_id(client);
        let mut params_map = Map::new();
        params_map.insert("name".into(), Value::String(name));
        params_map.insert("arguments".into(), arguments);
        let body = make_request(req_id, "prompts/get", Value::Object(params_map));
        let (job_id, token) = self.runtime.register_external(JobKind::McpRequest, None);
        client.pending_external.insert(
            req_id,
            PendingExternal {
                method: "prompts/get".to_owned(),
                cache_key: None,
                awaiters: vec![Awaiter { job_id, token }],
            },
        );
        if let Err(e) = send_frame_to(&self.supervisor, client, &body) {
            if let Some(p) = client.pending_external.remove(&req_id) {
                let msg = format!("get_prompt: {e}");
                for a in p.awaiters {
                    self.runtime.complete_external_failed(a.job_id, msg.clone());
                }
            }
            return Err(e);
        }
        Ok(job_id)
    }

    /// Send a JSON-RPC notification to `id`. Fire-and-forget.
    pub fn send_notification(
        &mut self,
        id: McpServerId,
        method: impl Into<String>,
        params: Value,
    ) -> Result<(), String> {
        let method = method.into();
        let client = self
            .clients
            .get_mut(&id)
            .ok_or_else(|| format!("unknown server: {id}"))?;
        if matches!(
            client.state,
            McpClientState::Stopped { .. } | McpClientState::Crashed { .. }
        ) {
            return Err(format!(
                "server {id} is not running (state: {})",
                state_label(&client.state)
            ));
        }
        let body = make_notification(&method, params);
        send_frame_to(&self.supervisor, client, &body)?;
        Ok(())
    }

    /// Reply to a server-initiated request.
    pub fn send_response(
        &mut self,
        id: McpServerId,
        request_id: Value,
        result: Result<Value, McpError>,
    ) -> Result<(), String> {
        let client = self
            .clients
            .get_mut(&id)
            .ok_or_else(|| format!("unknown server: {id}"))?;
        let body = make_response(request_id, result);
        send_frame_to(&self.supervisor, client, &body)?;
        Ok(())
    }

    /// One pass: drain process events into per-server frame buffers,
    /// parse frames, dispatch to per-server state. Apply restart
    /// policy on terminations. Also drain any user-cancelled handles
    /// so abandoned requests do not retain bookkeeping past the tick
    /// in which they were cancelled.
    pub fn tick(&mut self) {
        let server_ids: Vec<McpServerId> = self.clients.keys().copied().collect();
        for sid in server_ids {
            self.drain_process_events(sid);
            self.drain_cancelled_externals(sid);
            self.maybe_terminate_shutting_down(sid);
            self.maybe_restart(sid);
        }
    }

    /// Pass-3 finding 1: a server that has not exited within the
    /// shutdown grace window after stdin close gets SIGTERM, then
    /// SIGKILL after a second grace window. The server's actual exit
    /// lands as the next `Exited` / `Signaled` event in the
    /// supervisor's queue.
    fn maybe_terminate_shutting_down(&mut self, sid: McpServerId) {
        let now = Instant::now();
        let escalation = {
            let Some(client) = self.clients.get(&sid) else {
                return;
            };
            if !matches!(client.state, McpClientState::ShuttingDown) {
                return;
            }
            client.shutdown_escalation
        };
        match escalation {
            Some(ShutdownEscalation::SigtermAt(at)) if at <= now => {
                let pid = self.clients.get(&sid).and_then(|c| c.process);
                if let Some(pid) = pid {
                    let _ = self.supervisor.borrow_mut().terminate(pid);
                }
                if let Some(client) = self.clients.get_mut(&sid) {
                    client.shutdown_escalation =
                        Some(ShutdownEscalation::SigkillAt(now + self.shutdown_grace));
                }
            }
            Some(ShutdownEscalation::SigkillAt(at)) if at <= now => {
                let pid = self.clients.get(&sid).and_then(|c| c.process);
                if let Some(pid) = pid {
                    let _ = self.supervisor.borrow_mut().signal(pid, Signal::SIGKILL);
                }
                if let Some(client) = self.clients.get_mut(&sid) {
                    client.shutdown_escalation = None;
                }
            }
            _ => {}
        }
    }

    /// Walk the per-client `pending_external` table per awaiter,
    /// looking for cancelled handles. Pass-2 (M9.2) finding 2: each
    /// awaiter has its own [`CancellationToken`], so cancellation is
    /// per-handle rather than per-request.
    ///
    /// Behaviour:
    /// - An awaiter whose token is flipped is removed from its
    ///   entry and settled as Cancelled. The in-flight wire request
    ///   continues; other awaiters on the same entry continue to
    ///   wait for the response.
    /// - When the last awaiter on an entry cancels, the entry is
    ///   removed and the cache (if any) is cleaned up via
    ///   [`Self::settle_in_flight_with`] with no remaining awaiters.
    ///   T M9.3: at this point we also send `notifications/cancelled`
    ///   to the server (best-effort) and record the request id in
    ///   the client's `cancelled_rids` set so a late response —
    ///   the cancel/response race — is silently dropped rather
    ///   than emitting a "response for unknown request id"
    ///   `ProtocolError`.
    fn drain_cancelled_externals(&mut self, sid: McpServerId) {
        // First pass: per-awaiter cancellation drain. Collect the
        // job ids of cancelled awaiters and any entries whose
        // awaiter list is now empty.
        let (per_awaiter_cancellations, abandoned_entries): (Vec<JobId>, Vec<DrainedPending>) = {
            let Some(client) = self.clients.get_mut(&sid) else {
                return;
            };
            let mut cancelled_awaiters: Vec<JobId> = Vec::new();
            let mut abandoned: Vec<DrainedPending> = Vec::new();
            client.pending_external.retain(|rid, p| {
                let mut still_pending: Vec<Awaiter> = Vec::with_capacity(p.awaiters.len());
                for a in p.awaiters.drain(..) {
                    if a.token.is_cancelled() {
                        cancelled_awaiters.push(a.job_id);
                    } else {
                        still_pending.push(a);
                    }
                }
                p.awaiters = still_pending;
                if p.awaiters.is_empty() {
                    // Last awaiter cancelled — drop the entry and
                    // route through settle_in_flight_with so the
                    // cache state transitions correctly. The
                    // already-cancelled awaiters are settled
                    // individually below; settle_in_flight_with sees
                    // an empty awaiter list and only does cache
                    // cleanup.
                    abandoned.push((p.cache_key.clone(), *rid, Vec::new()));
                    false
                } else {
                    true
                }
            });
            (cancelled_awaiters, abandoned)
        };
        // Settle the per-awaiter cancellations directly: each one
        // already saw its token flipped, so we just mark the runtime
        // job cancelled. (settle_in_flight_with would also work but
        // would settle the entire entry; we want per-awaiter here.)
        for job_id in per_awaiter_cancellations {
            self.runtime.complete_external_cancelled(job_id);
        }
        // T M9.3: for entries fully abandoned by cancellation, send
        // `notifications/cancelled` to the server so it can release
        // any tied-up resources. Best-effort: a server that has
        // already died returns Err, which we ignore. Per-awaiter
        // cancellations (where siblings still wait) do NOT send the
        // notification — the server's work is still wanted.
        for (_cache_key, rid, _) in &abandoned_entries {
            self.send_cancellation_notification(sid, *rid);
            // Record the rid so a late response is dropped silently.
            if let Some(client) = self.clients.get_mut(&sid) {
                client.cancelled_rids.insert(*rid);
            }
        }
        // For entries whose awaiter list emptied entirely: route
        // through settle_in_flight_with so cache state transitions
        // are uniform with other failure paths.
        for (cache_key, rid, awaiters) in abandoned_entries {
            self.settle_in_flight_with(cache_key, rid, &awaiters, SettleOutcome::Cancelled);
        }
    }

    /// Send `notifications/cancelled { requestId, reason }` to the
    /// server for `rid` on `sid`. Best-effort: errors (e.g. the
    /// server died between the cancel and this call) are ignored.
    /// T M9.3.
    fn send_cancellation_notification(&self, sid: McpServerId, rid: u64) {
        let Some(client) = self.clients.get(&sid) else {
            return;
        };
        // Don't bother trying if the server isn't accepting writes.
        if matches!(
            client.state,
            McpClientState::Stopped { .. }
                | McpClientState::Crashed { .. }
                | McpClientState::ShuttingDown
        ) {
            return;
        }
        let body = make_notification(
            "notifications/cancelled",
            json!({
                "requestId": rid,
                "reason": "cancelled by pmacs handle"
            }),
        );
        let _ = send_frame_to(&self.supervisor, client, &body);
    }

    /// Canonical settlement for an in-flight external request.
    ///
    /// Settles the primary awaiter and any sibling awaiters with the
    /// same outcome, then updates the resource cache: a successful
    /// outcome for a request whose cache entry still references this
    /// `request_id` transitions `InFlight` → `Cached`. Anything else
    /// (failure, cancellation, or invalidation that re-dispatched
    /// under a new id) leaves the cache entry absent.
    ///
    /// T M9.2 design rule: the `InFlight` → `Cached` transition only
    /// happens on success of the request the cache currently
    /// references; every other terminal state results in
    /// `InFlight` → Absent. Centralising the rule here means the four
    /// settlement call sites (response handler, cancellation drain,
    /// `on_exit`, restart cancellation) cannot drift apart.
    fn settle_in_flight_with(
        &mut self,
        cache_key: Option<(McpServerId, String)>,
        request_id: u64,
        awaiters: &[Awaiter],
        outcome: SettleOutcome,
    ) {
        // Settle every awaiter. Clone the value per awaiter; the
        // runtime's bus-encoded delivery already requires owned
        // values. An empty awaiter list (from
        // `drain_cancelled_externals`'s "last awaiter cancelled"
        // path) is fine — we still need to update the cache below.
        //
        // Cancellation is checked again at settlement time, not only
        // in `drain_cancelled_externals`: a response and a user
        // cancellation can both be queued before this manager tick.
        // In that race, the cancelled handle must still settle as
        // Cancelled rather than receiving the response.
        match &outcome {
            SettleOutcome::Ok(value) => {
                for a in awaiters {
                    if a.token.is_cancelled() {
                        self.runtime.complete_external_cancelled(a.job_id);
                    } else {
                        self.runtime.complete_external_ok(a.job_id, value.clone());
                    }
                }
            }
            SettleOutcome::Failed(msg) => {
                for a in awaiters {
                    if a.token.is_cancelled() {
                        self.runtime.complete_external_cancelled(a.job_id);
                    } else {
                        self.runtime.complete_external_failed(a.job_id, msg.clone());
                    }
                }
            }
            SettleOutcome::Cancelled => {
                for a in awaiters {
                    self.runtime.complete_external_cancelled(a.job_id);
                }
            }
        }
        // Update cache.
        let Some(key) = cache_key else {
            return;
        };
        let in_flight_for_us = matches!(
            self.resource_cache.get(&key),
            Some(ResourceCacheState::InFlight { request_id: rid }) if *rid == request_id
        );
        if !in_flight_for_us {
            // Cache entry was invalidated, replaced by a newer
            // in-flight request, or was never installed (race during
            // a wire-write rollback). Don't touch it.
            return;
        }
        match outcome {
            SettleOutcome::Ok(value) => {
                self.resource_cache
                    .insert(key, ResourceCacheState::Cached { result: value });
            }
            SettleOutcome::Failed(_) | SettleOutcome::Cancelled => {
                self.resource_cache.remove(&key);
            }
        }
    }

    fn drain_process_events(&mut self, sid: McpServerId) {
        let Some(client) = self.clients.get_mut(&sid) else {
            return;
        };
        let Some(pid) = client.process else {
            return;
        };
        let mut sup = self.supervisor.borrow_mut();
        let events = sup.take_events(pid);
        drop(sup);
        for ev in events {
            self.handle_process_event(sid, ev);
        }
    }

    fn handle_process_event(&mut self, sid: McpServerId, ev: ProcessEvent) {
        match ev.kind {
            ProcessEventKind::Started { pid } => self.on_started(sid, pid, ev.at),
            ProcessEventKind::Stdout(bytes) => {
                if let Some(client) = self.clients.get_mut(&sid) {
                    client.stdout.extend(&bytes);
                }
                self.parse_frames(sid);
            }
            ProcessEventKind::Stderr(bytes) => {
                self.push_event(sid, ev.at, McpEventKind::Stderr(bytes));
            }
            ProcessEventKind::Ansi(_) => {
                self.push_event(
                    sid,
                    ev.at,
                    McpEventKind::ProtocolError {
                        message: "supervisor emitted ANSI events for pipe-mode MCP process".into(),
                    },
                );
            }
            ProcessEventKind::Exited { code } => {
                // Pass3-2 finding 5: distinguish clean exits from
                // crashes. Code 0 is clean; non-zero is a crash even
                // if the process exited "normally". OnCrash policy
                // restarts only on the latter.
                self.on_exit(sid, ev.at, format!("exit code {code}"), code == 0);
            }
            ProcessEventKind::Signaled { signal } => {
                // A signal-killed process is never clean — even
                // SIGTERM during user-initiated `stop` lands here as
                // !clean, but `was_shutdown` overrides the policy
                // path so the server transitions to Stopped anyway.
                self.on_exit(sid, ev.at, format!("signal {signal}"), false);
            }
            ProcessEventKind::Crashed { error } => {
                self.on_exit(sid, ev.at, format!("crashed: {error}"), false);
            }
            ProcessEventKind::Restarting { .. } => {
                // The supervisor's own restart accounting is unused
                // here (we run with RestartPolicy::Never on the
                // supervisor side). If a Restarting event ever
                // arrives, log it as a protocol-layer surprise.
                self.push_event(
                    sid,
                    ev.at,
                    McpEventKind::ProtocolError {
                        message: "supervisor emitted Restarting under MCP-managed process".into(),
                    },
                );
            }
        }
    }

    fn on_started(&mut self, sid: McpServerId, pid: u32, at: Instant) {
        let init_request_id = {
            let Some(client) = self.clients.get_mut(&sid) else {
                return;
            };
            let req_id = next_request_id(client);
            client.state = McpClientState::Initializing {
                init_request_id: req_id,
                started: at,
            };
            client
                .pending_internal
                .insert(req_id, "initialize".to_owned());
            req_id
        };
        let body = build_initialize(init_request_id);
        if let Some(client) = self.clients.get(&sid) {
            if let Err(e) = send_frame_to(&self.supervisor, client, &body) {
                self.push_event(
                    sid,
                    at,
                    McpEventKind::ProtocolError {
                        message: format!("failed to send initialize: {e}"),
                    },
                );
            }
        }
        self.push_event(sid, at, McpEventKind::Started { pid });
    }

    fn on_exit(&mut self, sid: McpServerId, at: Instant, reason: String, clean: bool) {
        let (terminal_kind, restart, cancelled_externals) = {
            let Some(client) = self.clients.get_mut(&sid) else {
                return;
            };
            let was_shutdown = matches!(client.state, McpClientState::ShuttingDown);
            // Pass-2 finding 5: clean exits go to Stopped (not
            // Crashed), and the OnCrash policy does not respawn
            // them. Non-clean exits go to Crashed unless they happen
            // during a user-initiated stop, where SIGTERM/SIGKILL
            // may be the expected shutdown path.
            let terminal_kind = if was_shutdown || clean {
                TerminalKind::Stopped
            } else {
                TerminalKind::Crashed
            };
            client.state = match terminal_kind {
                TerminalKind::Stopped => McpClientState::Stopped { ended: at },
                TerminalKind::Crashed => McpClientState::Crashed {
                    reason: reason.clone(),
                    ended: at,
                },
            };
            if let Some(pid) = client.process.take() {
                self.process_to_server.remove(&pid);
                let _ = self.supervisor.borrow_mut().forget(pid);
            }
            // Drain in-flight external requests; the server died
            // without responding to any of them. Every awaiter
            // wakes with a structured cancelled error. T M9.2:
            // route through `settle_in_flight_with` so awaiters
            // and cache state transitions are uniform.
            let cancelled: Vec<DrainedPending> = client
                .pending_external
                .drain()
                .map(|(rid, p)| (p.cache_key, rid, p.awaiters))
                .collect();
            // Drop internal initialize bookkeeping without ceremony;
            // it has no Lua-visible counterpart.
            client.pending_internal.clear();
            // T M9.3: clear the cancelled-rid set; the server is
            // gone, no responses are coming anyway.
            client.cancelled_rids.clear();
            let restart = should_restart(client.spec.restart, clean, was_shutdown);
            (terminal_kind, restart, cancelled)
        };
        for (cache_key, rid, awaiters) in cancelled_externals {
            self.settle_in_flight_with(cache_key, rid, &awaiters, SettleOutcome::Cancelled);
        }
        // Pass-2 (M9.2) finding 1: drop every cached entry for this
        // server. Once the server has reached a terminal state, a
        // stale `read_resource(sid, uri)` against `sid` must error
        // ("not ready") rather than return cache data left over
        // from before the exit.
        self.drop_resource_cache_for(sid);
        match terminal_kind {
            TerminalKind::Stopped => self.push_event(sid, at, McpEventKind::Stopped),
            TerminalKind::Crashed => self.push_event(sid, at, McpEventKind::Crashed { reason }),
        }
        if restart {
            if let Some(client) = self.clients.get_mut(&sid) {
                client.next_restart_at = Some(at + self.restart_backoff);
            }
        }
    }

    fn parse_frames(&mut self, sid: McpServerId) {
        loop {
            let frame = {
                let Some(client) = self.clients.get_mut(&sid) else {
                    return;
                };
                client.stdout.next_frame()
            };
            match frame {
                Some(body) => self.dispatch_inbound(sid, &body),
                None => return,
            }
        }
    }

    fn dispatch_inbound(&mut self, sid: McpServerId, body: &[u8]) {
        let now = Instant::now();
        let value: Value = match serde_json::from_slice(body) {
            Ok(v) => v,
            Err(e) => {
                self.push_event(
                    sid,
                    now,
                    McpEventKind::ProtocolError {
                        message: format!("invalid JSON in frame: {e}"),
                    },
                );
                return;
            }
        };
        let Value::Object(map) = value else {
            self.push_event(
                sid,
                now,
                McpEventKind::ProtocolError {
                    message: "frame body was not a JSON object".into(),
                },
            );
            return;
        };
        if let Some(jsonrpc) = map.get("jsonrpc").and_then(Value::as_str) {
            if jsonrpc != "2.0" {
                self.push_event(
                    sid,
                    now,
                    McpEventKind::ProtocolError {
                        message: format!("unexpected jsonrpc version: {jsonrpc:?}"),
                    },
                );
            }
        } else {
            self.push_event(
                sid,
                now,
                McpEventKind::ProtocolError {
                    message: "frame missing 'jsonrpc' field".into(),
                },
            );
        }
        let id = map.get("id").cloned();
        let method = map.get("method").and_then(Value::as_str).map(str::to_owned);
        let params = map.get("params").cloned().unwrap_or(Value::Null);
        let result = map.get("result").cloned();
        let error = map.get("error").map(McpError::from_value);
        match (id, method, result.is_some() || error.is_some()) {
            (Some(idv), None, true) => self.handle_response(sid, &idv, result, error, now),
            (Some(idv), Some(m), false) => self.handle_request(sid, idv, m, params, now),
            (None, Some(m), false) => self.handle_notification(sid, m, params, now),
            _ => self.push_event(
                sid,
                now,
                McpEventKind::ProtocolError {
                    message: "frame is neither request, response, nor notification".into(),
                },
            ),
        }
    }

    fn handle_response(
        &mut self,
        sid: McpServerId,
        idv: &Value,
        result: Option<Value>,
        error: Option<McpError>,
        now: Instant,
    ) {
        // Look up the request as either internal (`initialize`) or
        // external (a Lua-visible send_request).
        // Internal/external request-id namespaces share the same
        // counter so a given rid lands in at most one of the two
        // tables; the dual-lookup is just a flat-table search.
        let Some(rid) = idv.as_u64() else {
            self.push_event(
                sid,
                now,
                McpEventKind::ProtocolError {
                    message: format!("response id is not a u64: {idv:?}"),
                },
            );
            return;
        };
        let kind = {
            let Some(client) = self.clients.get_mut(&sid) else {
                return;
            };
            if let Some(method) = client.pending_internal.remove(&rid) {
                PendingKind::Internal(method)
            } else if let Some(p) = client.pending_external.remove(&rid) {
                PendingKind::External(p)
            } else if client.cancelled_rids.remove(&rid) {
                // T M9.3: cancel/response race. We sent
                // `notifications/cancelled` for this request; a
                // late response arrived anyway. Drop it silently —
                // by definition no awaiter is listening (the entry
                // was abandoned by cancellation), and emitting
                // `ProtocolError` here would be log spam for an
                // expected outcome.
                return;
            } else {
                PendingKind::Unknown
            }
        };
        match kind {
            PendingKind::Unknown => {
                self.push_event(
                    sid,
                    now,
                    McpEventKind::ProtocolError {
                        message: format!("response for unknown request id {rid}"),
                    },
                );
            }
            PendingKind::Internal(method) if method == "initialize" => {
                self.handle_initialize_response(sid, result, error, now);
            }
            PendingKind::Internal(other) => {
                // Internal request the manager doesn't know how to
                // handle. Pass-3 finding 1 removed `shutdown` from
                // this set (MCP stdio has no protocol shutdown
                // message); only `initialize` is internal now. Any
                // other entry would be a manager bug.
                self.push_event(
                    sid,
                    now,
                    McpEventKind::ProtocolError {
                        message: format!(
                            "internal pending entry for unrecognised method {other:?}"
                        ),
                    },
                );
            }
            PendingKind::External(p) => {
                // Settle the awaiters (one or more, attached via M3.5
                // coalescing for `read_resource`; always one for
                // `send_request` and `invoke_tool`). The `Response`
                // event still fires for callers observing the raw
                // event stream, but the canonical Lua-side delivery
                // path is the awaited handle.
                //
                // T M9.3: per-method translation of MCP "tool
                // errored" responses. A `tools/call` response with
                // `isError: true` is a tool-level failure and must
                // propagate as a Lua error, not as an Ok with the
                // flag set. Other methods pass through with the
                // raw result.
                let outcome = if let Some(err) = error.as_ref() {
                    SettleOutcome::Failed(format!("[{}] {}", err.code, err.message))
                } else if p.method == "tools/call"
                    && let Some(result_obj) = result.as_ref()
                    && result_obj
                        .get("isError")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                {
                    SettleOutcome::Failed(extract_tool_error_message(result_obj))
                } else {
                    SettleOutcome::Ok(result.clone().unwrap_or(Value::Null))
                };
                self.settle_in_flight_with(p.cache_key.clone(), rid, &p.awaiters, outcome);
                self.push_event(
                    sid,
                    now,
                    McpEventKind::Response {
                        id: rid,
                        result: result.unwrap_or(Value::Null),
                        error,
                        method: p.method,
                    },
                );
            }
        }
    }

    /// Handle the response to the manager-internal `initialize`
    /// request. Pass-2 findings 2 + 3:
    ///
    /// - **Finding 2**: a JSON-RPC `error` reply means the server
    ///   refused to initialize. We emit a `ProtocolError` + terminate
    ///   the process rather than masquerading as Initialized.
    /// - **Finding 3**: the server's `protocolVersion` echo is
    ///   validated against [`SUPPORTED_PROTOCOL_VERSIONS`]. An
    ///   unsupported version is a fatal protocol mismatch per the
    ///   MCP lifecycle spec; we terminate without sending
    ///   `notifications/initialized`.
    fn handle_initialize_response(
        &mut self,
        sid: McpServerId,
        result: Option<Value>,
        error: Option<McpError>,
        now: Instant,
    ) {
        if let Some(err) = error {
            let message = format!("server refused initialize: [{}] {}", err.code, err.message);
            self.push_event(
                sid,
                now,
                McpEventKind::ProtocolError {
                    message: message.clone(),
                },
            );
            self.terminate_after_protocol_error(sid);
            return;
        }
        let result_val = result.unwrap_or(Value::Null);
        let server_pv = result_val
            .get("protocolVersion")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let pv_acceptable = server_pv
            .as_deref()
            .is_some_and(|v| SUPPORTED_PROTOCOL_VERSIONS.contains(&v));
        if !pv_acceptable {
            let reported = server_pv.as_deref().unwrap_or("<missing>");
            let supported = SUPPORTED_PROTOCOL_VERSIONS.join(", ");
            self.push_event(
                sid,
                now,
                McpEventKind::ProtocolError {
                    message: format!(
                        "server reported unsupported protocolVersion {reported:?}; pmacs supports {{{supported}}}"
                    ),
                },
            );
            self.terminate_after_protocol_error(sid);
            return;
        }
        // Pass-3 finding 2: the MCP `initialize` result is required
        // to include a `capabilities` object. A missing or non-object
        // value is a protocol violation; we terminate rather than
        // transitioning to Initialized with a defaulted-Null
        // capabilities table that downstream consumers can't
        // distinguish from a real "no capabilities advertised".
        let caps_value = result_val.get("capabilities");
        let caps = match caps_value {
            Some(v) if v.is_object() => v.clone(),
            _ => {
                let observed = caps_value.map_or_else(
                    || "<missing>".to_owned(),
                    |v| {
                        match v {
                            Value::Null => "null",
                            Value::Bool(_) => "boolean",
                            Value::Number(_) => "number",
                            Value::String(_) => "string",
                            Value::Array(_) => "array",
                            Value::Object(_) => "object",
                        }
                        .to_owned()
                    },
                );
                self.push_event(
                    sid,
                    now,
                    McpEventKind::ProtocolError {
                        message: format!(
                            "initialize result missing required capabilities object (got {observed})"
                        ),
                    },
                );
                self.terminate_after_protocol_error(sid);
                return;
            }
        };
        let server_info = result_val.get("serverInfo").cloned();
        // Send `notifications/initialized` per the MCP spec.
        let body = make_notification("notifications/initialized", json!({}));
        if let Some(client) = self.clients.get(&sid) {
            let _ = send_frame_to(&self.supervisor, client, &body);
        }
        if let Some(client) = self.clients.get_mut(&sid) {
            client.state = McpClientState::Initialized {
                capabilities: caps.clone(),
                server_info,
                protocol_version: server_pv,
                initialized_at: now,
            };
        }
        self.push_event(sid, now, McpEventKind::Initialized { capabilities: caps });
    }

    /// Terminate the running generation after a fatal protocol
    /// violation. The process exit will land in the next tick as a
    /// `Crashed` event, which the configured restart policy then
    /// processes per [`should_restart`]. This is the MCP-equivalent
    /// of the LSP layer's "frame violations are unrecoverable on the
    /// same byte stream" path.
    fn terminate_after_protocol_error(&mut self, sid: McpServerId) {
        if let Some(client) = self.clients.get(&sid) {
            if let Some(pid) = client.process {
                let _ = self.supervisor.borrow_mut().terminate(pid);
            }
        }
    }

    fn handle_request(
        &mut self,
        sid: McpServerId,
        idv: Value,
        method: String,
        params: Value,
        now: Instant,
    ) {
        // Surface the request to the consumer; pmacs doesn't
        // synthesize a default reply (M9.5+ wires per-method handling).
        self.push_event(
            sid,
            now,
            McpEventKind::Request {
                id: idv,
                method,
                params,
            },
        );
    }

    fn handle_notification(
        &mut self,
        sid: McpServerId,
        method: String,
        params: Value,
        now: Instant,
    ) {
        // T M9.5: if the method is subscribed via on_notification,
        // queue (server, params) for Lua-side dispatch on the next
        // tick. The raw event still fires through push_event so
        // callers using events_take continue to see notifications.
        if self.notification_subscriptions.contains(&method) {
            self.notification_queue
                .entry(method.clone())
                .or_default()
                .push((sid, params.clone()));
        }
        self.push_event(sid, now, McpEventKind::Notification { method, params });
    }

    fn maybe_restart(&mut self, sid: McpServerId) {
        let now = Instant::now();
        let restart_now = {
            let Some(client) = self.clients.get(&sid) else {
                return;
            };
            let McpClientState::Crashed { .. } = &client.state else {
                return;
            };
            match client.next_restart_at {
                Some(at) => at <= now,
                None => false,
            }
        };
        if !restart_now {
            return;
        }
        let attempt = {
            let Some(client) = self.clients.get_mut(&sid) else {
                return;
            };
            client.attempt + 1
        };
        self.push_event(sid, now, McpEventKind::Restarting { attempt });
        let mut client = self.clients.remove(&sid).expect("checked existence above");
        // Pass-2 finding 4: surface spawn failures rather than
        // swallowing them. If the executable disappears, cwd is
        // invalid, etc., emit Crashed with the spawn error and
        // re-arm next_restart_at per policy. The retry loop is
        // bounded only by the user dropping the manager or stopping
        // the server; for v0.1 that's acceptable, since "binary
        // gone" is a configuration error worth reporting loudly.
        if let Err(err) = self.start_generation(sid, &mut client) {
            client.state = McpClientState::Crashed {
                reason: format!("respawn failed: {err}"),
                ended: now,
            };
            client.next_restart_at = if should_restart(client.spec.restart, false, false) {
                Some(now + self.restart_backoff)
            } else {
                None
            };
            self.clients.insert(sid, client);
            self.push_event(
                sid,
                now,
                McpEventKind::Crashed {
                    reason: format!("respawn failed: {err}"),
                },
            );
            return;
        }
        self.clients.insert(sid, client);
    }

    fn push_event(&mut self, sid: McpServerId, at: Instant, kind: McpEventKind) {
        let event = McpEvent {
            server: sid,
            kind,
            at,
        };
        self.pending.entry(sid).or_default().push(event);
    }

    /// Drain the pending event queue for `sid`. Returns an empty vec
    /// for unknown ids.
    pub fn take_events(&mut self, sid: McpServerId) -> Vec<McpEvent> {
        self.pending.remove(&sid).unwrap_or_default()
    }

    /// Drain every pending event across every server. Returns events
    /// sorted by enqueue time.
    pub fn take_all_events(&mut self) -> Vec<McpEvent> {
        let mut all = Vec::new();
        for (_id, mut evs) in std::mem::take(&mut self.pending) {
            all.append(&mut evs);
        }
        all.sort_by_key(|e| e.at);
        all
    }

    /// Iterator over every server id, in arbitrary order.
    pub fn ids(&self) -> impl Iterator<Item = McpServerId> + '_ {
        self.clients.keys().copied()
    }

    /// Server state for `sid`, or `None` if the id is unknown.
    #[must_use]
    pub fn state(&self, sid: McpServerId) -> Option<&McpClientState> {
        self.clients.get(&sid).map(|c| &c.state)
    }

    /// Server spec for `sid`, or `None` if the id is unknown.
    #[must_use]
    pub fn spec(&self, sid: McpServerId) -> Option<&McpServerSpec> {
        self.clients.get(&sid).map(|c| &c.spec)
    }

    /// Server capabilities for `sid`, or `None` if the id is unknown
    /// or the server isn't initialized.
    #[must_use]
    pub fn capabilities(&self, sid: McpServerId) -> Option<&Value> {
        self.clients.get(&sid).and_then(McpClient::capabilities)
    }

    /// Server-reported `serverInfo` from the initialize response, if
    /// the server provided one.
    #[must_use]
    pub fn server_info(&self, sid: McpServerId) -> Option<&Value> {
        match self.state(sid)? {
            McpClientState::Initialized { server_info, .. } => server_info.as_ref(),
            _ => None,
        }
    }

    /// Server-reported `protocolVersion`, if any.
    #[must_use]
    pub fn protocol_version(&self, sid: McpServerId) -> Option<&str> {
        match self.state(sid)? {
            McpClientState::Initialized {
                protocol_version, ..
            } => protocol_version.as_deref(),
            _ => None,
        }
    }

    /// Cumulative spawn-attempt count for `sid`. 1 = first spawn,
    /// 2 = first restart, ...
    #[must_use]
    pub fn attempt(&self, sid: McpServerId) -> Option<u32> {
        self.clients.get(&sid).map(|c| c.attempt)
    }

    /// Forget about `sid`. Server must already be in a terminal state.
    pub fn forget(&mut self, sid: McpServerId) -> Result<(), String> {
        let client = self
            .clients
            .get(&sid)
            .ok_or_else(|| format!("unknown server: {sid}"))?;
        if !matches!(
            client.state,
            McpClientState::Stopped { .. } | McpClientState::Crashed { .. }
        ) {
            return Err(format!("server {sid} is not in a terminal state"));
        }
        self.clients.remove(&sid);
        self.pending.remove(&sid);
        // Pass-2 (M9.2) finding 1: belt-and-braces. `on_exit`
        // already cleared the cache when the server reached a
        // terminal state; this second sweep handles the (rare)
        // case where forget is called against a server that
        // somehow accumulated cache entries after on_exit, and
        // makes the post-forget invariant ("no cache for sid")
        // explicit at this call site too.
        self.drop_resource_cache_for(sid);
        Ok(())
    }

    /// Initiate shutdown for every server. Mirrors
    /// [`crate::lsp::LspManager::shutdown_all`].
    pub fn shutdown_all(&mut self) {
        let ids: Vec<McpServerId> = self.clients.keys().copied().collect();
        for id in ids {
            let _ = self.stop(id);
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn next_request_id(client: &mut McpClient) -> u64 {
    let id = client.next_request_id;
    client.next_request_id += 1;
    id
}

fn state_label(state: &McpClientState) -> &'static str {
    match state {
        McpClientState::Starting => "starting",
        McpClientState::Initializing { .. } => "initializing",
        McpClientState::Initialized { .. } => "initialized",
        McpClientState::ShuttingDown => "shutting-down",
        McpClientState::Stopped { .. } => "stopped",
        McpClientState::Crashed { .. } => "crashed",
    }
}

/// Translate an [`McpClientState`] to a stable `&'static str` label.
/// Used by the Lua surface and tests so the spelling of state names
/// is locked down in one place.
#[must_use]
pub fn state_label_for(state: &McpClientState) -> &'static str {
    state_label(state)
}

/// Should the manager respawn after this exit? Pass-2 finding 5:
/// `OnCrash` matches the documented "signal or non-zero exit"
/// contract — clean exits do not trigger a restart. `Always`
/// restarts regardless. A user-initiated `stop` (`was_shutdown`)
/// always wins: even `Always` does not respawn when we asked the
/// server to shut down.
fn should_restart(policy: McpRestartPolicy, clean: bool, was_shutdown: bool) -> bool {
    if was_shutdown {
        return false;
    }
    match policy {
        McpRestartPolicy::Never => false,
        McpRestartPolicy::OnCrash => !clean,
        McpRestartPolicy::Always => true,
    }
}

#[derive(Copy, Clone, Debug)]
enum TerminalKind {
    Stopped,
    Crashed,
}

fn make_request(id: u64, method: &str, params: Value) -> Vec<u8> {
    let mut body = Map::new();
    body.insert("jsonrpc".into(), Value::String("2.0".into()));
    body.insert("id".into(), id.into());
    body.insert("method".into(), Value::String(method.to_owned()));
    body.insert("params".into(), params);
    let bytes = serde_json::to_vec(&Value::Object(body)).expect("json serialize");
    encode_frame(&bytes)
}

fn make_notification(method: &str, params: Value) -> Vec<u8> {
    let mut body = Map::new();
    body.insert("jsonrpc".into(), Value::String("2.0".into()));
    body.insert("method".into(), Value::String(method.to_owned()));
    body.insert("params".into(), params);
    let bytes = serde_json::to_vec(&Value::Object(body)).expect("json serialize");
    encode_frame(&bytes)
}

/// Extract a human-readable error message from a `tools/call`
/// `isError: true` response. T M9.3.
///
/// MCP's `content` array may have multiple parts (text, image, etc.).
/// The Failed outcome carries a single string, so:
/// - Text parts are concatenated in order with newline separators.
/// - Non-text parts are replaced with `[non-text content omitted]`
///   placeholders, preserving their position so users can see where
///   in the message the omitted content was.
/// - Order is preserved exactly as the server returned it; we don't
///   reorder, drop, or collapse adjacent parts.
///
/// Lua callers needing structured access to the raw `{isError,
/// content}` table use `pmacs.mcp.send_request("tools/call", ...)`
/// to bypass the translator.
fn extract_tool_error_message(result: &Value) -> String {
    let Some(content) = result.get("content").and_then(Value::as_array) else {
        return "tool errored (no content provided)".to_owned();
    };
    let parts: Vec<String> = content
        .iter()
        .map(|part| {
            let kind = part.get("type").and_then(Value::as_str);
            if kind == Some("text") {
                part.get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned()
            } else {
                "[non-text content omitted]".to_owned()
            }
        })
        .collect();
    if parts.is_empty() {
        "tool errored (empty content)".to_owned()
    } else {
        parts.join("\n")
    }
}

fn make_response(id: Value, result: Result<Value, McpError>) -> Vec<u8> {
    let mut body = Map::new();
    body.insert("jsonrpc".into(), Value::String("2.0".into()));
    body.insert("id".into(), id);
    match result {
        Ok(v) => {
            body.insert("result".into(), v);
        }
        Err(e) => {
            let mut err_obj = Map::new();
            err_obj.insert("code".into(), e.code.into());
            err_obj.insert("message".into(), Value::String(e.message));
            if let Some(d) = e.data {
                err_obj.insert("data".into(), d);
            }
            body.insert("error".into(), Value::Object(err_obj));
        }
    }
    let bytes = serde_json::to_vec(&Value::Object(body)).expect("json serialize");
    encode_frame(&bytes)
}

fn send_frame_to(
    supervisor: &crate::lua_bindings::SharedProcessSupervisor,
    client: &McpClient,
    framed: &[u8],
) -> Result<(), String> {
    let pid = client
        .process
        .ok_or_else(|| "server has no live process".to_owned())?;
    supervisor.borrow_mut().write_stdin(pid, framed)
}

/// Build the `initialize` request body for the v0.1 MCP client. The
/// payload advertises the conservative client capability set
/// (currently empty) plus pmacs's clientInfo. Mirrors LSP's
/// `build_initialize` in shape; the field set is MCP-specific.
///
/// Pass-2 finding 3: `protocolVersion` is the latest revision pmacs
/// supports per [`PREFERRED_PROTOCOL_VERSION`]; servers respond with
/// the version they negotiated and pmacs validates it against
/// [`SUPPORTED_PROTOCOL_VERSIONS`] in
/// [`McpManager::handle_initialize_response`].
fn build_initialize(request_id: u64) -> Vec<u8> {
    let params = json!({
        "protocolVersion": PREFERRED_PROTOCOL_VERSION,
        "capabilities": default_client_capabilities(),
        "clientInfo": {
            "name": "pmacs",
            "version": env!("CARGO_PKG_VERSION"),
        },
    });
    make_request(request_id, "initialize", params)
}

/// Conservative default MCP client capabilities. v0.1 advertises
/// nothing (no roots/list, no sampling); M9.5+ may add fields as
/// concrete client-side features land.
fn default_client_capabilities() -> Value {
    json!({})
}

// ---------------------------------------------------------------------------
// SharedMcpManager
// ---------------------------------------------------------------------------

/// Main-thread shared handle, mirroring
/// [`crate::lsp::SharedLspManager`].
pub type SharedMcpManager = Rc<RefCell<McpManager>>;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ndjson_parser_extracts_complete_lines() {
        let mut p = NdjsonParser::new();
        p.extend(b"{\"a\":1}\n{\"b\":2}\n");
        assert_eq!(p.next_frame().as_deref(), Some(&b"{\"a\":1}"[..]));
        assert_eq!(p.next_frame().as_deref(), Some(&b"{\"b\":2}"[..]));
        assert!(p.next_frame().is_none());
    }

    #[test]
    fn ndjson_parser_buffers_partial_line() {
        let mut p = NdjsonParser::new();
        p.extend(b"{\"a\":");
        assert!(p.next_frame().is_none());
        p.extend(b"1}\n");
        assert_eq!(p.next_frame().as_deref(), Some(&b"{\"a\":1}"[..]));
    }

    #[test]
    fn ndjson_parser_skips_empty_lines() {
        let mut p = NdjsonParser::new();
        p.extend(b"\n\n{\"a\":1}\n");
        assert_eq!(p.next_frame().as_deref(), Some(&b"{\"a\":1}"[..]));
        assert!(p.next_frame().is_none());
    }

    #[test]
    fn ndjson_parser_strips_crlf() {
        let mut p = NdjsonParser::new();
        p.extend(b"{\"a\":1}\r\n");
        assert_eq!(p.next_frame().as_deref(), Some(&b"{\"a\":1}"[..]));
    }

    #[test]
    fn encode_frame_appends_newline() {
        assert_eq!(encode_frame(b"{\"x\":1}"), b"{\"x\":1}\n");
    }

    #[test]
    fn server_id_is_monotonic() {
        let a = McpServerId::next();
        let b = McpServerId::next();
        assert!(a.raw() < b.raw());
    }
}
