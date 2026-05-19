// lsp.rs --- T M4.5 LSP client core.

//! Language Server Protocol client core. Spec §5.2 wire-format
//! channel reused for JSON-RPC 2.0 over `Content-Length`-framed
//! pipes. Sits on top of [`crate::process::ProcessSupervisor`]
//! (M4.4) for spawn / signal / restart / I/O; the supervisor's
//! per-pipe reader thread provides the parallelism --- the LSP
//! layer never reads from a pipe on the main thread.
//!
//! # Topology
//!
//! ```text
//!   main thread          supervisor (M4.4)         LSP server
//!   -----------          -------------------       -------------
//!   LspManager      ---> spawn + write_stdin --->  child stdin
//!     ^                                            child stdout
//!     +-- frame parse <--- Stdout(Vec<u8>) <------ reader thread
//!     +-- frame parse <--- Stderr(Vec<u8>) <------ reader thread
//! ```
//!
//! The supervisor's reader thread already chunks pipe output off the
//! main thread; the LSP layer just buffers those chunks per server
//! and parses `Content-Length` frames out of them in [`LspClient::pump`].
//! Parsing is incremental --- a frame split across two `Stdout` events
//! is reassembled without re-allocating.
//!
//! # Lifecycle
//!
//! ```text
//!   Starting --(supervisor: Started)--> Initializing
//!     --(initialize response)--> Initialized { capabilities }
//!     --(shutdown sent + exit)--> Stopped
//!     --(unexpected exit / signal)--> Crashed
//!     --(restart policy)--> Initializing  (new pid)
//! ```
//!
//! Every transition emits an [`LspEvent`] visible through
//! [`LspManager::take_events`] / [`LspManager::take_all_events`].
//! Crashes restart automatically when the spec's [`LspRestartPolicy`]
//! says so; the restart re-runs `initialize` from scratch (LSP
//! servers do not survive their process).
//!
//! # Concurrency
//!
//! A single [`LspManager`] lives on the main thread inside the
//! editor's `Rc<RefCell<...>>`. It mutates one [`ProcessSupervisor`]
//! it shares with `pmacs.process.*`. All inbound bytes arrive on
//! supervisor reader threads (one per pipe); all outbound writes go
//! through `ProcessSupervisor::write_stdin` synchronously from the
//! main thread. Per-server traffic is small (LSP is request-driven,
//! kilobytes per second) so the synchronous write is acceptable for
//! v0.1; M5 may push outbound to a writer thread if profiling shows
//! it matters.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use crate::process::{
    ProcessEvent, ProcessEventKind, ProcessId, ProcessMode, ProcessSpec, ProcessState,
    RestartPolicy, Termination,
};

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

/// Stable identifier for one managed LSP server. Allocated in
/// monotonic order.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct LspServerId(u64);

impl LspServerId {
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

impl std::fmt::Display for LspServerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LspServerId({})", self.0)
    }
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Spec for one LSP server.
#[derive(Clone, Debug)]
pub struct LspServerSpec {
    /// Human-readable label. Surfaced in events, the modeline, and
    /// the `*lsp*` buffer (M4.8).
    pub label: String,
    /// Language id for `didOpen` / `didChange` (e.g. `"rust"`,
    /// `"lua"`). LSP servers use this string to dispatch
    /// language-specific behavior.
    pub language_id: String,
    /// Program to execute. PATH-resolved unless absolute.
    pub command: String,
    /// Argument vector.
    pub args: Vec<String>,
    /// Working directory for the spawn. Also the default `rootUri`
    /// if [`Self::root_uri`] is `None`.
    pub cwd: Option<PathBuf>,
    /// Override for the root URI sent in `initialize`. `None` →
    /// derive from `cwd` (or the editor's cwd if both are unset).
    pub root_uri: Option<String>,
    /// Environment overrides for the child process.
    pub env: Vec<(String, String)>,
    /// Optional `initializationOptions` sent in the `initialize`
    /// request. Free-form per server; pmacs marshalls it into JSON
    /// from a Lua table.
    pub init_options: Option<Value>,
    /// Optional client-side capabilities override sent in
    /// `initialize`. `None` falls back to a conservative built-in
    /// default (text-sync full, hover, completion, definition).
    pub capabilities: Option<Value>,
    /// Restart policy for the server. Mirrors
    /// [`crate::process::RestartPolicy`].
    pub restart: LspRestartPolicy,
}

impl LspServerSpec {
    /// Construct a spec with the bare-minimum fields.
    #[must_use]
    pub fn new(
        label: impl Into<String>,
        language_id: impl Into<String>,
        command: impl Into<String>,
    ) -> Self {
        Self {
            label: label.into(),
            language_id: language_id.into(),
            command: command.into(),
            args: Vec::new(),
            cwd: None,
            root_uri: None,
            env: Vec::new(),
            init_options: None,
            capabilities: None,
            restart: LspRestartPolicy::OnCrash,
        }
    }

    fn to_process_spec(&self) -> ProcessSpec {
        let mut p = ProcessSpec::new(format!("lsp:{}", self.label), &self.command);
        p.args.clone_from(&self.args);
        p.cwd.clone_from(&self.cwd);
        p.env.clone_from(&self.env);
        p.mode = ProcessMode::Pipes;
        // We handle restart at the LSP layer (because we need to
        // re-run `initialize` after each restart); the supervisor
        // never restarts behind our back.
        p.restart = RestartPolicy::Never;
        p
    }
}

/// What to do when the LSP server process terminates unexpectedly.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LspRestartPolicy {
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

/// LSP client state machine. One instance per managed server.
#[derive(Clone, Debug)]
pub enum LspClientState {
    /// Process spawning; supervisor hasn't reported `Started` yet.
    Starting,
    /// Process is alive; `initialize` request is in flight.
    Initializing {
        /// JSON-RPC id of the in-flight `initialize` request.
        init_request_id: u64,
        /// When the server process started.
        started: Instant,
    },
    /// `initialize` has been answered; `initialized` notification
    /// has been sent. The server is ready for arbitrary requests.
    Initialized {
        /// `ServerCapabilities` returned by `initialize`. The full
        /// JSON is preserved; consumers (M4.6+) project the bits
        /// they need.
        capabilities: Value,
        /// `serverInfo` from the initialize result, if reported.
        server_info: Option<Value>,
        /// When the server reached this state.
        initialized_at: Instant,
    },
    /// `shutdown` request has been sent; the client is waiting for
    /// the response (after which it sends `exit`).
    ShuttingDown {
        /// Id of the in-flight shutdown request, or `None` once the
        /// shutdown response has been observed and the client is
        /// waiting for the process to exit.
        shutdown_request_id: Option<u64>,
    },
    /// Process exited cleanly after a deliberate `exit` notification.
    Stopped {
        /// When the process exited.
        ended: Instant,
    },
    /// Process died unexpectedly. The supervisor's [`Termination`]
    /// is preserved so the modeline / status surface can report it.
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

/// One LSP-layer event. Visible to the editor via
/// [`LspManager::take_events`] and to Lua via
/// `pmacs.lsp.events_take`.
#[derive(Clone, Debug)]
pub struct LspEvent {
    /// Server the event belongs to.
    pub server: LspServerId,
    /// What happened.
    pub kind: LspEventKind,
    /// When the event was generated (monotonic).
    pub at: Instant,
}

/// Discriminator over LSP-layer event kinds.
#[derive(Clone, Debug)]
pub enum LspEventKind {
    /// Process spawned and is running. The `initialize` request was
    /// just sent.
    Started {
        /// OS pid of the new generation.
        pid: u32,
    },
    /// `initialize` response was received and `initialized`
    /// notification was sent.
    Initialized {
        /// `ServerCapabilities` from the response.
        capabilities: Value,
    },
    /// Server-originated notification (e.g.
    /// `textDocument/publishDiagnostics`, `window/logMessage`).
    Notification {
        /// JSON-RPC method name.
        method: String,
        /// Params payload (already JSON-decoded).
        params: Value,
    },
    /// Server-originated request (rare in v0.1; needed for
    /// `window/workDoneProgress/create`, `client/registerCapability`).
    /// Pmacs replies with a default error for any request it doesn't
    /// know how to handle so the protocol stays alive.
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
        error: Option<LspError>,
        /// Method of the original request, copied here for
        /// convenience so consumers don't have to track ids.
        method: String,
    },
    /// `shutdown` response received and `exit` notification was
    /// sent. The next `Process` event observed will be the actual
    /// exit.
    ShuttingDown,
    /// Server process exited cleanly. Final state for this
    /// generation.
    Stopped,
    /// Server process died (signal or non-zero exit not preceded by
    /// `shutdown`).
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
    /// chatter here. Treated as opaque for now; M4.8 stitches it
    /// into `*lsp*`.
    Stderr(Vec<u8>),
    /// A protocol violation was observed and recovered from.
    /// Reported separately so the status surface can flag the
    /// server as misbehaving.
    ProtocolError {
        /// Display-formatted description of what went wrong.
        message: String,
    },
}

/// JSON-RPC error object (`{code, message, data?}`).
#[derive(Clone, Debug)]
pub struct LspError {
    /// Numeric error code. LSP defines several standard codes
    /// (-32000 to -32099 for server errors, etc.).
    pub code: i64,
    /// Human-readable message.
    pub message: String,
    /// Optional `data` field, free-form JSON.
    pub data: Option<Value>,
}

impl LspError {
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

/// Incremental `Content-Length`-framed parser for JSON-RPC over
/// pipes. Holds a single `Vec<u8>` buffer that grows with appended
/// bytes and shrinks (via memmove) when complete frames are
/// extracted. Designed for the throughput LSPs actually produce
/// (kilobytes/sec); the buffer is rarely larger than the largest
/// pending frame.
#[derive(Default)]
pub struct FrameParser {
    buf: Vec<u8>,
}

impl FrameParser {
    /// New empty parser.
    #[must_use]
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Append `chunk` to the internal buffer.
    pub fn extend(&mut self, chunk: &[u8]) {
        self.buf.extend_from_slice(chunk);
    }

    /// Pop one complete frame off the front of the buffer if one is
    /// available, returning its body bytes. Returns `Ok(None)` when
    /// no complete frame is buffered yet, and `Err` for an
    /// unrecoverable framing error (in which case the caller is
    /// expected to re-spawn the server; staying on the same byte
    /// stream after a frame violation is unsafe).
    pub fn next_frame(&mut self) -> Result<Option<Vec<u8>>, FrameError> {
        // Find the header terminator `\r\n\r\n`.
        let Some(header_end) = find_subsequence(&self.buf, b"\r\n\r\n") else {
            return Ok(None);
        };
        let header_str =
            std::str::from_utf8(&self.buf[..header_end]).map_err(|_| FrameError::HeaderNotUtf8)?;
        let mut content_length: Option<usize> = None;
        for line in header_str.split("\r\n") {
            if line.is_empty() {
                continue;
            }
            let (key, value) = line
                .split_once(':')
                .ok_or_else(|| FrameError::MalformedHeader(line.to_owned()))?;
            if key.trim().eq_ignore_ascii_case("content-length") {
                let n: usize = value
                    .trim()
                    .parse()
                    .map_err(|_| FrameError::MalformedHeader(line.to_owned()))?;
                content_length = Some(n);
            }
            // Other headers (e.g. `Content-Type: ...`) are ignored.
        }
        let body_len = content_length.ok_or(FrameError::MissingContentLength)?;
        let frame_total = header_end + 4 + body_len;
        if self.buf.len() < frame_total {
            return Ok(None);
        }
        let body = self.buf[header_end + 4..frame_total].to_vec();
        // Consume the frame from the front of the buffer.
        self.buf.drain(..frame_total);
        Ok(Some(body))
    }
}

/// Fatal framing error.
#[derive(Debug)]
pub enum FrameError {
    /// Header bytes weren't valid UTF-8 (LSP requires ASCII headers
    /// but UTF-8 is the same in that range).
    HeaderNotUtf8,
    /// A header line was malformed (no colon, or `Content-Length`
    /// value didn't parse as a number).
    MalformedHeader(String),
    /// The header block contained no `Content-Length` field; we
    /// have no way to know how many bytes to read.
    MissingContentLength,
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HeaderNotUtf8 => write!(f, "frame header was not valid UTF-8"),
            Self::MalformedHeader(line) => write!(f, "malformed header line: {line:?}"),
            Self::MissingContentLength => write!(f, "frame missing Content-Length header"),
        }
    }
}

impl std::error::Error for FrameError {}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Encode a JSON-RPC body as a complete `Content-Length`-framed
/// message. Returns the bytes ready to write to the server's stdin.
#[must_use]
pub fn encode_frame(body: &[u8]) -> Vec<u8> {
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    let mut out = Vec::with_capacity(header.len() + body.len());
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(body);
    out
}

// ---------------------------------------------------------------------------
// LspClient
// ---------------------------------------------------------------------------

/// One managed LSP server: process + JSON-RPC framing + state
/// machine. Owned by [`LspManager`]; the supervisor handles process
/// I/O underneath.
pub struct LspClient {
    spec: LspServerSpec,
    state: LspClientState,
    process: Option<ProcessId>,
    stdout: FrameParser,
    /// JSON-RPC request id counter. Monotonically increasing.
    next_request_id: u64,
    /// Pending requests we sent and are waiting on a response for.
    /// Maps request id → method (so the response event can echo
    /// the method name back to the consumer).
    pending: HashMap<u64, String>,
    /// Cumulative spawn attempts (1 = first spawn, 2 = first
    /// restart, ...).
    attempt: u32,
    /// When the manager should attempt the next restart, if any.
    next_restart_at: Option<Instant>,
}

impl LspClient {
    fn new(spec: LspServerSpec) -> Self {
        Self {
            spec,
            state: LspClientState::Starting,
            process: None,
            stdout: FrameParser::new(),
            next_request_id: 1,
            pending: HashMap::new(),
            attempt: 0,
            next_restart_at: None,
        }
    }

    /// Current spec.
    #[must_use]
    pub fn spec(&self) -> &LspServerSpec {
        &self.spec
    }

    /// Current lifecycle state.
    #[must_use]
    pub fn state(&self) -> &LspClientState {
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
            LspClientState::Initialized { capabilities, .. } => Some(capabilities),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// LspManager
// ---------------------------------------------------------------------------

/// Per-editor registry of [`LspClient`]s. Owns no threads of its
/// own; ticks via [`Self::tick`] from the run loop.
pub struct LspManager {
    supervisor: crate::lua_bindings::SharedProcessSupervisor,
    clients: HashMap<LspServerId, LspClient>,
    /// Reverse lookup: which server is which process.
    process_to_server: HashMap<ProcessId, LspServerId>,
    /// Per-server pending event buffer.
    pending: HashMap<LspServerId, Vec<LspEvent>>,
    /// Constant restart back-off (mirrors the supervisor's default).
    restart_backoff: Duration,
    /// T M4.6 diagnostic store. Populated as the manager handles
    /// `textDocument/publishDiagnostics` notifications. Shared with
    /// every [`crate::diag::DiagnosticView`] attached to a buffer.
    diag_store: crate::diag::SharedDiagStore,
    /// T M4.7 completion store. Populated when a
    /// `textDocument/completion` response lands.
    completion_store: crate::completion::SharedCompletionStore,
    /// T M4.7 hover store.
    hover_store: crate::hover::SharedHoverStore,
    /// T M4.7 signature-help store.
    signature_store: crate::signature::SharedSignatureStore,
    /// T M4.12 definition store. Populated when a
    /// `textDocument/definition` response lands.
    definition_store: crate::definition::SharedDefinitionStore,
    /// T M4.12 formatting store. Populated when a
    /// `textDocument/formatting` response lands.
    formatting_store: crate::formatting::SharedFormattingStore,
    /// Per-server request id → response routing target.
    /// `request_completion` etc. record an entry here; `handle_response`
    /// consumes it to absorb the response into the correct store.
    pending_routes: HashMap<(LspServerId, u64), ResponseRoute>,
    /// T M4.8 status tracker. Folds every emitted [`LspEvent`] into a
    /// per-server [`crate::lsp_status::LspStatus`] for the modeline /
    /// `*lsp*` buffer.
    status_tracker: crate::lsp_status::LspStatusTracker,
    /// T M4.9: `(project_root, language_id)` → server id. Drives the
    /// "LSP runs per-project, not per-buffer" invariant. Roots are
    /// stored as [`PathBuf`] so callers don't have to canonicalise
    /// at every call site --- [`canonicalize_root_for_scope`] does
    /// it once before insert.
    project_servers: HashMap<(PathBuf, String), LspServerId>,
}

/// Routing target for a deferred response. Indexed by `(server, request_id)`
/// in [`LspManager::pending_routes`].
#[derive(Clone, Debug)]
enum ResponseRoute {
    /// Absorb response into [`crate::completion::CompletionStore`] at
    /// `(server, uri)`.
    Completion { uri: String },
    /// Absorb response into [`crate::hover::HoverStore`] at
    /// `(server, uri)`.
    Hover { uri: String },
    /// Absorb response into [`crate::signature::SignatureStore`] at
    /// `(server, uri)`.
    Signature { uri: String },
    /// Absorb response into [`crate::definition::DefinitionStore`] at
    /// `(server, uri)`.
    Definition { uri: String },
    /// Absorb response into [`crate::formatting::FormattingStore`] at
    /// `(server, uri)`.
    Formatting { uri: String },
}

impl LspManager {
    /// Construct a fresh manager wired to `supervisor`.
    #[must_use]
    pub fn new(supervisor: crate::lua_bindings::SharedProcessSupervisor) -> Self {
        Self {
            supervisor,
            clients: HashMap::new(),
            process_to_server: HashMap::new(),
            pending: HashMap::new(),
            restart_backoff: Duration::from_millis(500),
            diag_store: crate::diag::make_shared_store(),
            completion_store: crate::completion::make_shared_store(),
            hover_store: crate::hover::make_shared_store(),
            signature_store: crate::signature::make_shared_store(),
            definition_store: crate::definition::make_shared_store(),
            formatting_store: crate::formatting::make_shared_store(),
            pending_routes: HashMap::new(),
            status_tracker: crate::lsp_status::LspStatusTracker::new(),
            project_servers: HashMap::new(),
        }
    }

    /// Shared diagnostic store. Cheap to clone; used by every
    /// [`crate::diag::DiagnosticView`] attached to a buffer.
    #[must_use]
    pub fn diag_store(&self) -> crate::diag::SharedDiagStore {
        self.diag_store.clone()
    }

    /// Shared completion store. Used by every
    /// [`crate::completion::CompletionView`].
    #[must_use]
    pub fn completion_store(&self) -> crate::completion::SharedCompletionStore {
        self.completion_store.clone()
    }

    /// Shared hover store. Used by every [`crate::hover::HoverView`].
    #[must_use]
    pub fn hover_store(&self) -> crate::hover::SharedHoverStore {
        self.hover_store.clone()
    }

    /// Shared signature-help store. Used by every
    /// [`crate::signature::SignatureView`].
    #[must_use]
    pub fn signature_store(&self) -> crate::signature::SharedSignatureStore {
        self.signature_store.clone()
    }

    /// Shared definition store (T M4.12).
    #[must_use]
    pub fn definition_store(&self) -> crate::definition::SharedDefinitionStore {
        self.definition_store.clone()
    }

    /// Shared formatting store (T M4.12).
    #[must_use]
    pub fn formatting_store(&self) -> crate::formatting::SharedFormattingStore {
        self.formatting_store.clone()
    }

    /// T M4.8: per-server status snapshot, derived from the LSP event
    /// stream. The modeline reads its label from this.
    #[must_use]
    pub fn status_for(&self, sid: LspServerId) -> Option<&crate::lsp_status::LspStatus> {
        self.status_tracker.get(sid)
    }

    /// T M4.8: short modeline label for `sid`, e.g. `"ready"`,
    /// `"idx"`, `"crashed"`. Falls back to `"?"` for unknown ids.
    #[must_use]
    pub fn modeline_label(&self, sid: LspServerId) -> &'static str {
        self.status_tracker.get(sid).map_or("?", |s| s.kind.label())
    }

    /// T M4.8: most recent error observed for `sid`, if any.
    #[must_use]
    pub fn last_error(&self, sid: LspServerId) -> Option<&crate::lsp_status::LspStatusError> {
        self.status_tracker
            .get(sid)
            .and_then(|s| s.last_error.as_ref())
    }

    /// T M4.8: render the contents of the `*lsp*` status buffer.
    #[must_use]
    pub fn status_buffer_text(&self) -> String {
        let now = Instant::now();
        let labels: HashMap<LspServerId, String> = self
            .clients
            .iter()
            .map(|(id, c)| (*id, c.spec.label.clone()))
            .collect();
        let caps: HashMap<LspServerId, Value> = self
            .clients
            .iter()
            .filter_map(|(id, c)| c.capabilities().cloned().map(|v| (*id, v)))
            .collect();
        crate::lsp_status::format_status_buffer(
            &self.status_tracker,
            |id| labels.get(&id).cloned(),
            |id| caps.get(&id).cloned(),
            now,
        )
    }

    /// Override the restart back-off. Test helper.
    pub fn set_restart_backoff(&mut self, d: Duration) {
        self.restart_backoff = d;
    }

    /// Spawn a server. Synchronous; the spawn itself is reported
    /// via the next tick's `Started` event (or `Crashed` if the
    /// supervisor refused the exec).
    pub fn spawn(&mut self, spec: LspServerSpec) -> Result<LspServerId, String> {
        let id = LspServerId::next();
        let mut client = LspClient::new(spec);
        self.start_generation(id, &mut client)?;
        self.clients.insert(id, client);
        Ok(id)
    }

    /// T M4.9: get-or-spawn one LSP server per `(project_root,
    /// language_id)` pair. If a healthy server is already serving
    /// the pair, returns its id; otherwise spawns one using
    /// `spec_template` (with `cwd` defaulted to `project_root` and
    /// `root_uri` defaulted to a `file://` URI) and records the
    /// mapping. Crashed and stopped servers are pruned before lookup
    /// so the caller never gets a dead handle.
    pub fn ensure_server_for_project(
        &mut self,
        project_root: impl Into<PathBuf>,
        language_id: impl Into<String>,
        spec_template: LspServerSpec,
    ) -> Result<LspServerId, String> {
        let project_root = canonicalize_root_for_scope(&project_root.into());
        let language_id = language_id.into();
        let key = (project_root.clone(), language_id.clone());
        // Drop the cached id if the server has died or been forgotten.
        if let Some(id) = self.project_servers.get(&key).copied() {
            let still_healthy = self.clients.get(&id).is_some_and(|c| {
                !matches!(
                    c.state,
                    LspClientState::Stopped { .. } | LspClientState::Crashed { .. }
                )
            });
            if still_healthy {
                return Ok(id);
            }
            self.project_servers.remove(&key);
        }
        // Materialise the spec: default cwd / root_uri from the project root.
        let mut spec = spec_template;
        spec.language_id = language_id;
        if spec.cwd.is_none() {
            spec.cwd = Some(project_root.clone());
        }
        if spec.root_uri.is_none() {
            spec.root_uri = Some(path_to_file_uri(&project_root));
        }
        let id = self.spawn(spec)?;
        self.project_servers.insert(key, id);
        Ok(id)
    }

    /// T M4.9: lookup-only. Returns the server id currently serving
    /// `(project_root, language_id)` if any.
    #[must_use]
    pub fn server_for_project(
        &self,
        project_root: &Path,
        language_id: &str,
    ) -> Option<LspServerId> {
        let canon = canonicalize_root_for_scope(project_root);
        self.project_servers
            .get(&(canon, language_id.to_owned()))
            .copied()
    }

    /// T M4.9: number of servers scoped to projects.
    #[must_use]
    pub fn project_scoped_server_count(&self) -> usize {
        self.project_servers.len()
    }

    fn start_generation(&mut self, id: LspServerId, client: &mut LspClient) -> Result<(), String> {
        client.attempt += 1;
        client.next_restart_at = None;
        client.stdout = FrameParser::new();
        client.pending.clear();
        // T M4.7: drop any pending response routes for this server.
        // Their request ids belong to the previous generation; the
        // new server starts request id numbering fresh.
        self.pending_routes.retain(|(sid, _), _| *sid != id);
        client.state = LspClientState::Starting;
        let proc_spec = client.spec.to_process_spec();
        let pid = self.supervisor.borrow_mut().spawn(proc_spec)?;
        client.process = Some(pid);
        self.process_to_server.insert(pid, id);
        Ok(())
    }

    /// Send `shutdown` + `exit` to the server, then forget it. The
    /// supervisor reaps the process; the manager removes the client
    /// once the exit lands. Idempotent: a server that's already
    /// stopped/crashed is removed without further protocol traffic.
    pub fn stop(&mut self, id: LspServerId) -> Result<(), String> {
        let Some(client) = self.clients.get_mut(&id) else {
            return Err(format!("unknown server: {id}"));
        };
        // Disable restart so a clean exit doesn't trigger a respawn.
        client.spec.restart = LspRestartPolicy::Never;
        client.next_restart_at = None;
        if let LspClientState::Initialized { .. } = &client.state {
            let req_id = next_request_id(client);
            let body = make_request(req_id, "shutdown", Value::Null);
            client.pending.insert(req_id, "shutdown".to_owned());
            client.state = LspClientState::ShuttingDown {
                shutdown_request_id: Some(req_id),
            };
            send_frame_to(&self.supervisor, client, &body)?;
        } else {
            // Not yet initialized (or already shutting down /
            // stopped / crashed). Skip the polite path: terminate
            // the process directly.
            if let Some(pid) = client.process {
                let _ = self.supervisor.borrow_mut().terminate(pid);
            }
            // Mark as stopping so the next exit observation cleans up.
            client.state = LspClientState::ShuttingDown {
                shutdown_request_id: None,
            };
        }
        Ok(())
    }

    /// Send a JSON-RPC request to `id`. Returns the JSON-RPC id;
    /// the response will surface as an [`LspEventKind::Response`]
    /// event tagged with the same id.
    pub fn send_request(
        &mut self,
        id: LspServerId,
        method: impl Into<String>,
        params: Value,
    ) -> Result<u64, String> {
        let method = method.into();
        let client = self
            .clients
            .get_mut(&id)
            .ok_or_else(|| format!("unknown server: {id}"))?;
        if !is_ready_for_request(&client.state) {
            return Err(format!(
                "server {id} is not ready for requests (state: {})",
                state_label(&client.state)
            ));
        }
        let req_id = next_request_id(client);
        let body = make_request(req_id, &method, params);
        client.pending.insert(req_id, method);
        send_frame_to(&self.supervisor, client, &body)?;
        Ok(req_id)
    }

    /// Send a JSON-RPC notification to `id`. Fire-and-forget; no
    /// response is expected.
    pub fn send_notification(
        &mut self,
        id: LspServerId,
        method: impl Into<String>,
        params: Value,
    ) -> Result<(), String> {
        let method = method.into();
        let client = self
            .clients
            .get_mut(&id)
            .ok_or_else(|| format!("unknown server: {id}"))?;
        // Notifications during init are allowed (the spec actually
        // *requires* `initialized` during the initializing phase),
        // but disallowed after Stopped/Crashed since stdin is gone.
        if matches!(
            client.state,
            LspClientState::Stopped { .. } | LspClientState::Crashed { .. }
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

    /// Send `textDocument/completion` for `uri` at `(line, col)`.
    /// The response is absorbed into the completion store at
    /// `(sid, uri)`; the same `LspEventKind::Response` event is also
    /// emitted so direct observers can see the raw payload.
    /// Returns the JSON-RPC request id.
    pub fn request_completion(
        &mut self,
        sid: LspServerId,
        uri: impl Into<String>,
        line: u32,
        col: u32,
    ) -> Result<u64, String> {
        let uri = uri.into();
        let params = json!({
            "textDocument": { "uri": uri.clone() },
            "position": { "line": line, "character": col }
        });
        let req_id = self.send_request(sid, "textDocument/completion", params)?;
        self.pending_routes
            .insert((sid, req_id), ResponseRoute::Completion { uri });
        Ok(req_id)
    }

    /// Send `textDocument/hover` for `uri` at `(line, col)`.
    pub fn request_hover(
        &mut self,
        sid: LspServerId,
        uri: impl Into<String>,
        line: u32,
        col: u32,
    ) -> Result<u64, String> {
        let uri = uri.into();
        let params = json!({
            "textDocument": { "uri": uri.clone() },
            "position": { "line": line, "character": col }
        });
        let req_id = self.send_request(sid, "textDocument/hover", params)?;
        self.pending_routes
            .insert((sid, req_id), ResponseRoute::Hover { uri });
        Ok(req_id)
    }

    /// Send `textDocument/signatureHelp` for `uri` at `(line, col)`.
    pub fn request_signature_help(
        &mut self,
        sid: LspServerId,
        uri: impl Into<String>,
        line: u32,
        col: u32,
    ) -> Result<u64, String> {
        let uri = uri.into();
        let params = json!({
            "textDocument": { "uri": uri.clone() },
            "position": { "line": line, "character": col }
        });
        let req_id = self.send_request(sid, "textDocument/signatureHelp", params)?;
        self.pending_routes
            .insert((sid, req_id), ResponseRoute::Signature { uri });
        Ok(req_id)
    }

    /// Send `textDocument/definition` for `uri` at `(line, col)`. The
    /// response is absorbed into the definition store at `(sid, uri)`.
    pub fn request_definition(
        &mut self,
        sid: LspServerId,
        uri: impl Into<String>,
        line: u32,
        col: u32,
    ) -> Result<u64, String> {
        let uri = uri.into();
        let params = json!({
            "textDocument": { "uri": uri.clone() },
            "position": { "line": line, "character": col }
        });
        let req_id = self.send_request(sid, "textDocument/definition", params)?;
        self.pending_routes
            .insert((sid, req_id), ResponseRoute::Definition { uri });
        Ok(req_id)
    }

    /// Send `textDocument/formatting` for `uri` with `tab_size` /
    /// `insert_spaces` formatting options. The response is absorbed
    /// into the formatting store at `(sid, uri)`.
    pub fn request_formatting(
        &mut self,
        sid: LspServerId,
        uri: impl Into<String>,
        tab_size: u32,
        insert_spaces: bool,
    ) -> Result<u64, String> {
        let uri = uri.into();
        let params = json!({
            "textDocument": { "uri": uri.clone() },
            "options": {
                "tabSize": tab_size,
                "insertSpaces": insert_spaces,
            }
        });
        let req_id = self.send_request(sid, "textDocument/formatting", params)?;
        self.pending_routes
            .insert((sid, req_id), ResponseRoute::Formatting { uri });
        Ok(req_id)
    }

    /// Reply to a server-initiated request.
    pub fn send_response(
        &mut self,
        id: LspServerId,
        request_id: Value,
        result: Result<Value, LspError>,
    ) -> Result<(), String> {
        let client = self
            .clients
            .get_mut(&id)
            .ok_or_else(|| format!("unknown server: {id}"))?;
        let body = make_response(request_id, result);
        send_frame_to(&self.supervisor, client, &body)?;
        Ok(())
    }

    /// One pass: drain process events into per-server frame
    /// buffers, parse frames, dispatch to per-server state. Apply
    /// restart policy on terminations.
    ///
    /// The supervisor's own `tick` must have already been called
    /// (or be called shortly before/after). [`crate::editor::EditorState::tick_lsp`]
    /// orders both.
    pub fn tick(&mut self) {
        // Snapshot ids first to avoid mutating the map during iter.
        // Frame parsing happens inline inside `drain_process_events`
        // (per Stdout event) so that state transitions from inbound
        // messages always order *before* subsequent Exit events in
        // the same supervisor batch.
        let server_ids: Vec<LspServerId> = self.clients.keys().copied().collect();
        for sid in server_ids {
            self.drain_process_events(sid);
            self.maybe_restart(sid);
        }
        // T M4.8: housekeeping for the status tracker (releases stale
        // `Degraded` flags). Snapshot states first so the closure
        // doesn't have to borrow self.
        let now = Instant::now();
        let states: HashMap<LspServerId, LspClientState> = self
            .clients
            .iter()
            .map(|(id, c)| (*id, c.state.clone()))
            .collect();
        self.status_tracker.tick(now, |id| states.get(&id).cloned());
    }

    /// Pull supervisor events for the server's process and feed
    /// stdout into the frame parser. Translates exit/signal/crash
    /// into LSP-layer state transitions.
    fn drain_process_events(&mut self, sid: LspServerId) {
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

    fn handle_process_event(&mut self, sid: LspServerId, ev: ProcessEvent) {
        match ev.kind {
            ProcessEventKind::Started { pid } => self.on_started(sid, pid, ev.at),
            ProcessEventKind::Stdout(bytes) => {
                // Buffer and parse inline so any state transitions
                // triggered by complete frames in this chunk happen
                // *before* any subsequent termination event in the
                // same supervisor batch. Without this ordering, an
                // `initialize` response that arrived just before
                // the server crashed would re-enter `Initialized`
                // after the `Exited` had already moved the client
                // to `Crashed`.
                if let Some(client) = self.clients.get_mut(&sid) {
                    client.stdout.extend(&bytes);
                }
                self.parse_frames(sid);
            }
            ProcessEventKind::Stderr(bytes) => {
                self.push_event(sid, ev.at, LspEventKind::Stderr(bytes));
            }
            ProcessEventKind::Ansi(_) => {
                self.push_event(
                    sid,
                    ev.at,
                    LspEventKind::ProtocolError {
                        message: "supervisor emitted ANSI events for pipe-mode LSP process".into(),
                    },
                );
            }
            ProcessEventKind::Exited { code } => {
                self.on_exit(sid, ev.at, format!("exit code {code}"), code == 0);
            }
            ProcessEventKind::Signaled { signal } => {
                self.on_exit(sid, ev.at, format!("signal {signal}"), false);
            }
            ProcessEventKind::Crashed { error } => {
                self.on_exit(sid, ev.at, format!("crashed: {error}"), false);
            }
            ProcessEventKind::Restarting { .. } => {
                // The supervisor's own restart accounting is unused
                // here (we run the LSP layer with RestartPolicy::Never
                // on the supervisor side). If a Restarting event ever
                // arrives, log it as a protocol-layer surprise.
                self.push_event(
                    sid,
                    ev.at,
                    LspEventKind::ProtocolError {
                        message: "supervisor emitted Restarting under LSP-managed process".into(),
                    },
                );
            }
        }
    }

    fn on_started(&mut self, sid: LspServerId, pid: u32, at: Instant) {
        let init_request_id = {
            let Some(client) = self.clients.get_mut(&sid) else {
                return;
            };
            let req_id = next_request_id(client);
            client.state = LspClientState::Initializing {
                init_request_id: req_id,
                started: at,
            };
            client.pending.insert(req_id, "initialize".to_owned());
            req_id
        };
        // Build the initialize request payload.
        let body = self.build_initialize(sid, init_request_id);
        if let Some(client) = self.clients.get(&sid)
            && let Err(e) = send_frame_to(&self.supervisor, client, &body)
        {
            self.push_event(
                sid,
                at,
                LspEventKind::ProtocolError {
                    message: format!("failed to send initialize: {e}"),
                },
            );
        }
        self.push_event(sid, at, LspEventKind::Started { pid });
    }

    fn build_initialize(&self, sid: LspServerId, request_id: u64) -> Vec<u8> {
        let client = self.clients.get(&sid).expect("sid has client");
        let cwd_path: Option<PathBuf> = client
            .spec
            .cwd
            .clone()
            .or_else(|| std::env::current_dir().ok());
        let root_uri = client
            .spec
            .root_uri
            .clone()
            .or_else(|| cwd_path.as_deref().map(path_to_file_uri));
        let process_id_value: Value = std::process::id().into();
        let capabilities = client
            .spec
            .capabilities
            .clone()
            .unwrap_or_else(default_capabilities);
        let mut params = Map::new();
        params.insert("processId".into(), process_id_value);
        if let Some(uri) = root_uri.as_ref() {
            params.insert("rootUri".into(), Value::String(uri.clone()));
            // rootPath is deprecated but still honored by some servers.
            if let Some(p) = cwd_path.as_ref() {
                params.insert("rootPath".into(), Value::String(p.display().to_string()));
            }
            // workspaceFolders so multi-root-aware servers are happy.
            params.insert(
                "workspaceFolders".into(),
                Value::Array(vec![json!({
                    "uri": uri,
                    "name": client.spec.label.clone(),
                })]),
            );
        } else {
            params.insert("rootUri".into(), Value::Null);
        }
        if let Some(opts) = client.spec.init_options.clone() {
            params.insert("initializationOptions".into(), opts);
        }
        params.insert("capabilities".into(), capabilities);
        params.insert(
            "clientInfo".into(),
            json!({
                "name": "pmacs",
                "version": env!("CARGO_PKG_VERSION"),
            }),
        );
        params.insert("trace".into(), Value::String("off".into()));
        make_request(request_id, "initialize", Value::Object(params))
    }

    fn on_exit(&mut self, sid: LspServerId, at: Instant, reason: String, _clean: bool) {
        let (was_shutdown, restart) = {
            let Some(client) = self.clients.get_mut(&sid) else {
                return;
            };
            let was_shutdown = matches!(client.state, LspClientState::ShuttingDown { .. });
            if was_shutdown {
                client.state = LspClientState::Stopped { ended: at };
            } else {
                client.state = LspClientState::Crashed {
                    reason: reason.clone(),
                    ended: at,
                };
            }
            // Drop the supervisor↔server mapping; the supervisor is
            // free to forget the pid now.
            if let Some(pid) = client.process.take() {
                self.process_to_server.remove(&pid);
                let _ = self.supervisor.borrow_mut().forget(pid);
            }
            (
                was_shutdown,
                !was_shutdown && should_restart(client.spec.restart),
            )
        };
        if was_shutdown {
            self.push_event(sid, at, LspEventKind::Stopped);
        } else {
            self.push_event(sid, at, LspEventKind::Crashed { reason });
        }
        if restart && let Some(client) = self.clients.get_mut(&sid) {
            client.next_restart_at = Some(at + self.restart_backoff);
        }
    }

    fn parse_frames(&mut self, sid: LspServerId) {
        loop {
            let frame_or_err = {
                let Some(client) = self.clients.get_mut(&sid) else {
                    return;
                };
                client.stdout.next_frame()
            };
            match frame_or_err {
                Ok(Some(body)) => self.dispatch_inbound(sid, &body),
                Ok(None) => return,
                Err(e) => {
                    let now = Instant::now();
                    self.push_event(
                        sid,
                        now,
                        LspEventKind::ProtocolError {
                            message: format!("frame error: {e}; restarting server"),
                        },
                    );
                    // Frame violations are unrecoverable on the same
                    // byte stream; terminate and let the restart
                    // policy bring things back if configured.
                    if let Some(client) = self.clients.get_mut(&sid)
                        && let Some(pid) = client.process
                    {
                        let _ = self.supervisor.borrow_mut().terminate(pid);
                    }
                    return;
                }
            }
        }
    }

    fn dispatch_inbound(&mut self, sid: LspServerId, body: &[u8]) {
        let now = Instant::now();
        let value: Value = match serde_json::from_slice(body) {
            Ok(v) => v,
            Err(e) => {
                self.push_event(
                    sid,
                    now,
                    LspEventKind::ProtocolError {
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
                LspEventKind::ProtocolError {
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
                    LspEventKind::ProtocolError {
                        message: format!("unexpected jsonrpc version: {jsonrpc:?}"),
                    },
                );
                // Continue: a permissive server might still produce
                // a usable message.
            }
        } else {
            self.push_event(
                sid,
                now,
                LspEventKind::ProtocolError {
                    message: "frame missing 'jsonrpc' field".into(),
                },
            );
        }
        let id = map.get("id").cloned();
        let method = map.get("method").and_then(Value::as_str).map(str::to_owned);
        let params = map.get("params").cloned().unwrap_or(Value::Null);
        let result = map.get("result").cloned();
        let error = map.get("error").map(LspError::from_value);
        match (id, method, result.is_some() || error.is_some()) {
            // Response: has id and (result or error), no method.
            (Some(idv), None, true) => self.handle_response(sid, &idv, result, error, now),
            // Request: has id and method.
            (Some(idv), Some(m), false) => self.handle_request(sid, idv, m, params, now),
            // Notification: no id, has method.
            (None, Some(m), false) => self.handle_notification(sid, m, params, now),
            // Anything else is a protocol violation we can't repair.
            _ => self.push_event(
                sid,
                now,
                LspEventKind::ProtocolError {
                    message: "frame is neither request, response, nor notification".into(),
                },
            ),
        }
    }

    fn handle_response(
        &mut self,
        sid: LspServerId,
        idv: &Value,
        result: Option<Value>,
        error: Option<LspError>,
        now: Instant,
    ) {
        let Some(rid) = idv.as_u64() else {
            self.push_event(
                sid,
                now,
                LspEventKind::ProtocolError {
                    message: format!("response id is not a u64: {idv:?}"),
                },
            );
            return;
        };
        let method = {
            let Some(client) = self.clients.get_mut(&sid) else {
                return;
            };
            client.pending.remove(&rid).unwrap_or_default()
        };
        if method.is_empty() {
            self.push_event(
                sid,
                now,
                LspEventKind::ProtocolError {
                    message: format!("response for unknown request id {rid}"),
                },
            );
            return;
        }
        // Special-case lifecycle responses: initialize and shutdown.
        if method == "initialize" {
            let result_val = result.clone().unwrap_or(Value::Null);
            let caps = result_val
                .get("capabilities")
                .cloned()
                .unwrap_or(Value::Null);
            let server_info = result_val.get("serverInfo").cloned();
            // Send `initialized` notification per the LSP spec.
            let body = make_notification("initialized", json!({}));
            if let Some(client) = self.clients.get(&sid) {
                let _ = send_frame_to(&self.supervisor, client, &body);
            }
            if let Some(client) = self.clients.get_mut(&sid) {
                client.state = LspClientState::Initialized {
                    capabilities: caps.clone(),
                    server_info,
                    initialized_at: now,
                };
            }
            self.push_event(sid, now, LspEventKind::Initialized { capabilities: caps });
            return;
        }
        if method == "shutdown" {
            // Send the `exit` notification and wait for the
            // process to actually exit (handled by on_exit).
            let body = make_notification("exit", Value::Null);
            if let Some(client) = self.clients.get(&sid) {
                let _ = send_frame_to(&self.supervisor, client, &body);
            }
            if let Some(client) = self.clients.get_mut(&sid) {
                client.state = LspClientState::ShuttingDown {
                    shutdown_request_id: None,
                };
            }
            self.push_event(sid, now, LspEventKind::ShuttingDown);
            return;
        }
        // T M4.7: routed responses (completion / hover / signature
        // help) get absorbed into the matching shared store before
        // surfacing as a generic `Response` event. Consumers
        // observing the event in the same tick see the fresh data.
        if let Some(route) = self.pending_routes.remove(&(sid, rid))
            && error.is_none()
            && let Some(value) = result.as_ref()
        {
            self.absorb_routed_response(sid, &route, value);
        }
        // Generic response.
        self.push_event(
            sid,
            now,
            LspEventKind::Response {
                id: rid,
                result: result.unwrap_or(Value::Null),
                error,
                method,
            },
        );
    }

    fn absorb_routed_response(&self, sid: LspServerId, route: &ResponseRoute, result: &Value) {
        let server_key = sid.raw().to_string();
        match route {
            ResponseRoute::Completion { uri } => {
                let resp = crate::completion::CompletionResponse::from_lsp_value(result);
                let key = crate::completion::CompletionKey::new(server_key, uri.clone());
                let mut guard = self
                    .completion_store
                    .lock()
                    .expect("completion store mutex poisoned");
                guard.set(key, resp);
            }
            ResponseRoute::Hover { uri } => {
                let key = crate::hover::HoverKey::new(server_key, uri.clone());
                let mut guard = self.hover_store.lock().expect("hover store mutex poisoned");
                if let Some(h) = crate::hover::Hover::from_lsp_value(result) {
                    guard.set(key, h);
                } else {
                    guard.clear(&key);
                }
            }
            ResponseRoute::Signature { uri } => {
                let help = crate::signature::SignatureHelp::from_lsp_value(result);
                let key = crate::signature::SignatureKey::new(server_key, uri.clone());
                let mut guard = self
                    .signature_store
                    .lock()
                    .expect("signature store mutex poisoned");
                guard.set(key, help);
            }
            ResponseRoute::Definition { uri } => {
                let resp = crate::definition::DefinitionResponse::from_lsp_value(result);
                let key = crate::definition::DefinitionKey::new(server_key, uri.clone());
                let mut guard = self
                    .definition_store
                    .lock()
                    .expect("definition store mutex poisoned");
                guard.set(key, resp);
            }
            ResponseRoute::Formatting { uri } => {
                let resp = crate::formatting::FormattingResponse::from_lsp_value(result);
                let key = crate::formatting::FormattingKey::new(server_key, uri.clone());
                let mut guard = self
                    .formatting_store
                    .lock()
                    .expect("formatting store mutex poisoned");
                guard.set(key, resp);
            }
        }
    }

    fn handle_request(
        &mut self,
        sid: LspServerId,
        idv: Value,
        method: String,
        params: Value,
        now: Instant,
    ) {
        // We don't synthesize a default error here --- expose the
        // request to the consumer (M4.6+ wires diagnostics, etc.)
        // and let it choose to reply via `send_response`. Until M4.6
        // ships replies for the requests we recognise, unknown
        // requests will simply linger; the LSP spec tolerates
        // delayed responses.
        self.push_event(
            sid,
            now,
            LspEventKind::Request {
                id: idv,
                method,
                params,
            },
        );
    }

    fn handle_notification(
        &mut self,
        sid: LspServerId,
        method: String,
        params: Value,
        now: Instant,
    ) {
        // T M4.6: intercept `textDocument/publishDiagnostics`
        // before forwarding the event so the diagnostic store is
        // updated synchronously --- a consumer that re-queries the
        // store from inside the same tick (e.g. a render observing
        // a fresh notification) sees the new diagnostics.
        if method == "textDocument/publishDiagnostics" {
            self.absorb_publish_diagnostics(&params);
        }
        self.push_event(sid, now, LspEventKind::Notification { method, params });
    }

    /// Parse `params` as a `PublishDiagnosticsParams` payload and
    /// update [`Self::diag_store`]. Malformed payloads (missing uri
    /// or non-array diagnostics) are ignored --- they're a
    /// server-side bug, not a fatal protocol error.
    fn absorb_publish_diagnostics(&self, params: &Value) {
        let Some(uri) = params.get("uri").and_then(Value::as_str) else {
            return;
        };
        let Some(arr) = params.get("diagnostics").and_then(Value::as_array) else {
            return;
        };
        let parsed: Vec<crate::diag::Diagnostic> = arr
            .iter()
            .filter_map(crate::diag::Diagnostic::from_lsp_value)
            .collect();
        let mut guard = self.diag_store.lock().expect("diag store mutex poisoned");
        guard.set(uri.to_owned(), parsed);
    }

    fn maybe_restart(&mut self, sid: LspServerId) {
        let now = Instant::now();
        let restart_now = {
            let Some(client) = self.clients.get(&sid) else {
                return;
            };
            let LspClientState::Crashed { .. } = &client.state else {
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
        self.push_event(sid, now, LspEventKind::Restarting { attempt });
        // Detach the entry temporarily so `start_generation` (which
        // also borrows `self.process_to_server`) doesn't conflict
        // with the `clients.get_mut` borrow.
        let mut client = self.clients.remove(&sid).expect("checked existence above");
        let _ = self.start_generation(sid, &mut client);
        self.clients.insert(sid, client);
    }

    fn push_event(&mut self, sid: LspServerId, at: Instant, kind: LspEventKind) {
        let event = LspEvent {
            server: sid,
            kind,
            at,
        };
        // Feed every event through the status tracker before
        // queueing it so a consumer that calls `status_for` after
        // `take_events` sees the post-event state.
        let client_state = self.clients.get(&sid).map(|c| c.state.clone());
        self.status_tracker.observe(&event, client_state.as_ref());
        self.pending.entry(sid).or_default().push(event);
    }

    /// Drain the pending event queue for `sid`. Returns an empty
    /// vec for unknown ids.
    pub fn take_events(&mut self, sid: LspServerId) -> Vec<LspEvent> {
        self.pending.remove(&sid).unwrap_or_default()
    }

    /// Drain every pending event across every server. Returns
    /// events sorted by enqueue time.
    pub fn take_all_events(&mut self) -> Vec<LspEvent> {
        let mut all = Vec::new();
        for (_id, mut evs) in std::mem::take(&mut self.pending) {
            all.append(&mut evs);
        }
        all.sort_by_key(|e| e.at);
        all
    }

    /// Iterator over every server id, in arbitrary order.
    pub fn ids(&self) -> impl Iterator<Item = LspServerId> + '_ {
        self.clients.keys().copied()
    }

    /// Server state for `sid`, or `None` if the id is unknown.
    #[must_use]
    pub fn state(&self, sid: LspServerId) -> Option<&LspClientState> {
        self.clients.get(&sid).map(|c| &c.state)
    }

    /// Server spec for `sid`, or `None` if the id is unknown.
    #[must_use]
    pub fn spec(&self, sid: LspServerId) -> Option<&LspServerSpec> {
        self.clients.get(&sid).map(|c| &c.spec)
    }

    /// Server capabilities for `sid`, or `None` if the id is unknown
    /// or the server isn't initialized.
    #[must_use]
    pub fn capabilities(&self, sid: LspServerId) -> Option<&Value> {
        self.clients.get(&sid).and_then(LspClient::capabilities)
    }

    /// Cumulative spawn-attempt count for `sid`. 1 = first spawn,
    /// 2 = first restart, ...
    #[must_use]
    pub fn attempt(&self, sid: LspServerId) -> Option<u32> {
        self.clients.get(&sid).map(|c| c.attempt)
    }

    /// Forget about `sid`. Server must already be in a terminal
    /// state.
    pub fn forget(&mut self, sid: LspServerId) -> Result<(), String> {
        let client = self
            .clients
            .get(&sid)
            .ok_or_else(|| format!("unknown server: {sid}"))?;
        if !matches!(
            client.state,
            LspClientState::Stopped { .. } | LspClientState::Crashed { .. }
        ) {
            return Err(format!("server {sid} is not in a terminal state"));
        }
        self.clients.remove(&sid);
        self.pending.remove(&sid);
        self.pending_routes.retain(|(s, _), _| *s != sid);
        self.status_tracker.forget(sid);
        // T M4.9: drop the project scoping so the next
        // ensure_server_for_project call spawns a fresh server.
        self.project_servers.retain(|_, v| *v != sid);
        Ok(())
    }

    /// Convenience: send `textDocument/didOpen` to `sid`.
    pub fn did_open(
        &mut self,
        sid: LspServerId,
        uri: impl Into<String>,
        version: i64,
        text: impl Into<String>,
    ) -> Result<(), String> {
        let language_id = self
            .clients
            .get(&sid)
            .map(|c| c.spec.language_id.clone())
            .ok_or_else(|| format!("unknown server: {sid}"))?;
        let params = json!({
            "textDocument": {
                "uri": uri.into(),
                "languageId": language_id,
                "version": version,
                "text": text.into(),
            }
        });
        self.send_notification(sid, "textDocument/didOpen", params)
    }

    /// Convenience: send `textDocument/didChange` to `sid` with full
    /// text replacement (the simplest sync mode; M5+ may add
    /// incremental sync).
    pub fn did_change_full(
        &mut self,
        sid: LspServerId,
        uri: impl Into<String>,
        version: i64,
        text: impl Into<String>,
    ) -> Result<(), String> {
        let params = json!({
            "textDocument": {
                "uri": uri.into(),
                "version": version,
            },
            "contentChanges": [{
                "text": text.into(),
            }],
        });
        self.send_notification(sid, "textDocument/didChange", params)
    }

    /// Convenience: send `textDocument/didClose` to `sid`.
    pub fn did_close(&mut self, sid: LspServerId, uri: impl Into<String>) -> Result<(), String> {
        let params = json!({
            "textDocument": { "uri": uri.into() },
        });
        self.send_notification(sid, "textDocument/didClose", params)
    }

    /// Initiate shutdown for every server. The supervisor's own
    /// shutdown will SIGTERM/SIGKILL anything that hasn't actually
    /// exited by the time the editor drops; this is the LSP-layer
    /// polite path.
    pub fn shutdown_all(&mut self) {
        let ids: Vec<LspServerId> = self.clients.keys().copied().collect();
        for id in ids {
            let _ = self.stop(id);
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn next_request_id(client: &mut LspClient) -> u64 {
    let id = client.next_request_id;
    client.next_request_id += 1;
    id
}

fn is_ready_for_request(state: &LspClientState) -> bool {
    matches!(state, LspClientState::Initialized { .. })
}

fn state_label(state: &LspClientState) -> &'static str {
    match state {
        LspClientState::Starting => "starting",
        LspClientState::Initializing { .. } => "initializing",
        LspClientState::Initialized { .. } => "initialized",
        LspClientState::ShuttingDown { .. } => "shutting-down",
        LspClientState::Stopped { .. } => "stopped",
        LspClientState::Crashed { .. } => "crashed",
    }
}

fn should_restart(policy: LspRestartPolicy) -> bool {
    matches!(policy, LspRestartPolicy::OnCrash | LspRestartPolicy::Always)
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

fn make_response(id: Value, result: Result<Value, LspError>) -> Vec<u8> {
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
    client: &LspClient,
    framed: &[u8],
) -> Result<(), String> {
    let pid = client
        .process
        .ok_or_else(|| "server has no live process".to_owned())?;
    supervisor.borrow_mut().write_stdin(pid, framed)
}

/// Conservative default LSP client capabilities. Covers the
/// transports M4.6/M4.7 will need (diagnostics push, completion,
/// hover, signature help, definition).
fn default_capabilities() -> Value {
    json!({
        "workspace": {
            "applyEdit": false,
            "configuration": false,
            "workspaceFolders": true,
            "didChangeConfiguration": { "dynamicRegistration": false },
        },
        "textDocument": {
            "synchronization": {
                "dynamicRegistration": false,
                "willSave": false,
                "willSaveWaitUntil": false,
                "didSave": true,
            },
            "completion": {
                "dynamicRegistration": false,
                "completionItem": {
                    "snippetSupport": false,
                    "documentationFormat": ["plaintext", "markdown"],
                },
            },
            "hover": {
                "dynamicRegistration": false,
                "contentFormat": ["plaintext", "markdown"],
            },
            "signatureHelp": { "dynamicRegistration": false },
            "definition": { "dynamicRegistration": false, "linkSupport": true },
            "formatting": { "dynamicRegistration": false },
            "publishDiagnostics": { "relatedInformation": true },
        },
    })
}

/// Canonicalise `root` for use as a key in the project-server map.
/// Falls back to the path as-given if canonicalisation fails (e.g.
/// the path doesn't exist on disk, common in unit tests).
fn canonicalize_root_for_scope(root: &Path) -> PathBuf {
    root.canonicalize().unwrap_or_else(|_| root.to_path_buf())
}

/// `file://` URI encoder. Byte-identical to
/// `builtin/runtime/lsp.lua`'s `file_uri_for` (same passthrough set),
/// so a URI built here keys into the same `DiagnosticStore` entry the
/// Lua LSP glue opened the document under. T M11.3 reuses this from
/// `crate::semantic_render` for the diagnostics projection — hence
/// `pub(crate)`.
pub(crate) fn path_to_file_uri(path: &std::path::Path) -> String {
    // Minimal file:// URI encoder: percent-encode anything outside
    // the LSP-friendly set. Adequate for v0.1 (paths in a typical
    // project root); a fuller URL crate would be overkill here.
    let mut out = String::from("file://");
    for ch in path.display().to_string().chars() {
        match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '/' | '-' | '_' | '.' | '~' | ':' => out.push(ch),
            _ => {
                use std::fmt::Write as _;
                let mut buf = [0u8; 4];
                for byte in ch.encode_utf8(&mut buf).as_bytes() {
                    let _ = write!(out, "%{byte:02X}");
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// SharedLspManager
// ---------------------------------------------------------------------------

/// Main-thread shared handle, mirroring [`crate::lua_bindings::SharedProcessSupervisor`].
pub type SharedLspManager = Rc<RefCell<LspManager>>;

// ---------------------------------------------------------------------------
// Termination helpers (used by Lua boundary)
// ---------------------------------------------------------------------------

/// Translate an [`LspClientState`] to a stable `&'static str` label.
/// Used by the Lua surface and tests so the spelling of state names
/// is locked down in one place.
#[must_use]
pub fn state_label_for(state: &LspClientState) -> &'static str {
    state_label(state)
}

/// True iff `state` is a terminal (won't transition without a
/// restart). Used by the Lua surface.
#[must_use]
pub fn is_terminal_state(state: &LspClientState) -> bool {
    matches!(
        state,
        LspClientState::Stopped { .. } | LspClientState::Crashed { .. }
    )
}

/// Match a [`Termination`] against an [`LspRestartPolicy`]. Exposed
/// for tests and consumers that inspect the supervisor directly.
#[must_use]
pub fn termination_warrants_restart(policy: LspRestartPolicy, termination: &Termination) -> bool {
    match (policy, termination) {
        (LspRestartPolicy::Never, _) => false,
        (LspRestartPolicy::Always, _)
        | (LspRestartPolicy::OnCrash, Termination::Signaled { .. } | Termination::Crashed { .. }) => {
            true
        }
        (LspRestartPolicy::OnCrash, Termination::Exited { code, .. }) => *code != 0,
    }
}

/// Helper for [`crate::editor::EditorState::tick_lsp`]. Returns
/// `true` iff `state` is `Initialized`.
#[must_use]
pub fn is_initialized(state: &LspClientState) -> bool {
    matches!(state, LspClientState::Initialized { .. })
}

/// Map a [`ProcessState`] back to an LSP-friendly string. Used by
/// the modeline status surface (M4.8) when no LSP-layer state is
/// available.
#[must_use]
pub fn process_state_label(state: &ProcessState) -> &'static str {
    match state {
        ProcessState::Starting => "process-starting",
        ProcessState::Running { .. } => "process-running",
        ProcessState::Exiting { .. } => "process-exiting",
        ProcessState::Terminated(_) => "process-terminated",
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_parser_extracts_a_complete_frame() {
        let mut p = FrameParser::new();
        let body = br#"{"jsonrpc":"2.0","method":"x"}"#;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
        bytes.extend_from_slice(body);
        p.extend(&bytes);
        let frame = p.next_frame().expect("parse").expect("some frame");
        assert_eq!(frame, body);
        assert!(p.next_frame().expect("parse").is_none());
    }

    #[test]
    fn frame_parser_handles_split_chunks() {
        let mut p = FrameParser::new();
        let body = br#"{"a":1}"#;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        // Feed byte-by-byte; the parser must reassemble.
        for byte in header.as_bytes() {
            p.extend(std::slice::from_ref(byte));
            assert!(p.next_frame().expect("ok").is_none());
        }
        for (i, byte) in body.iter().enumerate() {
            p.extend(std::slice::from_ref(byte));
            if i + 1 < body.len() {
                assert!(p.next_frame().expect("ok").is_none());
            }
        }
        let frame = p.next_frame().expect("ok").expect("complete");
        assert_eq!(frame, body);
    }

    #[test]
    fn frame_parser_extracts_two_back_to_back_frames() {
        let mut p = FrameParser::new();
        let mut bytes = Vec::new();
        for body in [&b"{\"a\":1}"[..], &b"{\"b\":2}"[..]] {
            bytes.extend_from_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
            bytes.extend_from_slice(body);
        }
        p.extend(&bytes);
        let f1 = p.next_frame().unwrap().unwrap();
        let f2 = p.next_frame().unwrap().unwrap();
        assert_eq!(f1, b"{\"a\":1}");
        assert_eq!(f2, b"{\"b\":2}");
        assert!(p.next_frame().unwrap().is_none());
    }

    #[test]
    fn frame_parser_rejects_malformed_header() {
        let mut p = FrameParser::new();
        p.extend(b"NotAHeader\r\n\r\n");
        let err = p.next_frame().unwrap_err();
        assert!(matches!(err, FrameError::MalformedHeader(_)));
    }

    #[test]
    fn frame_parser_rejects_missing_content_length() {
        let mut p = FrameParser::new();
        p.extend(b"Content-Type: utf-8\r\n\r\n{}");
        let err = p.next_frame().unwrap_err();
        assert!(matches!(err, FrameError::MissingContentLength));
    }

    #[test]
    fn encode_frame_round_trips() {
        let body = br#"{"jsonrpc":"2.0","id":1,"method":"x"}"#;
        let framed = encode_frame(body);
        let mut p = FrameParser::new();
        p.extend(&framed);
        let parsed = p.next_frame().unwrap().unwrap();
        assert_eq!(parsed, body);
    }

    #[test]
    fn make_request_includes_jsonrpc_id_and_method() {
        let bytes = make_request(7, "echo", json!({"x": 1}));
        let mut p = FrameParser::new();
        p.extend(&bytes);
        let body = p.next_frame().unwrap().unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["jsonrpc"], "2.0");
        assert_eq!(v["id"], 7);
        assert_eq!(v["method"], "echo");
        assert_eq!(v["params"]["x"], 1);
    }

    #[test]
    fn make_notification_omits_id() {
        let bytes = make_notification("ping", Value::Null);
        let mut p = FrameParser::new();
        p.extend(&bytes);
        let body = p.next_frame().unwrap().unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert!(v.get("id").is_none());
        assert_eq!(v["method"], "ping");
    }

    #[test]
    fn path_to_file_uri_encodes_spaces() {
        let p = std::path::Path::new("/tmp/a b/file.rs");
        let uri = path_to_file_uri(p);
        assert!(uri.starts_with("file:///tmp/a"));
        assert!(uri.contains("%20"));
        assert!(uri.ends_with("/file.rs"));
    }

    #[test]
    fn lsp_error_from_value_handles_optional_data() {
        let v = json!({"code": -32603, "message": "Internal", "data": {"trace": "stack"}});
        let e = LspError::from_value(&v);
        assert_eq!(e.code, -32603);
        assert_eq!(e.message, "Internal");
        assert!(e.data.is_some());
    }

    #[test]
    fn termination_policy_matrix() {
        let exit_clean = Termination::Exited {
            code: 0,
            started: Instant::now(),
            ended: Instant::now(),
        };
        let exit_nonzero = Termination::Exited {
            code: 7,
            started: Instant::now(),
            ended: Instant::now(),
        };
        let signaled = Termination::Signaled {
            signal: "SIGTERM".into(),
            started: Instant::now(),
            ended: Instant::now(),
        };
        assert!(!termination_warrants_restart(
            LspRestartPolicy::Never,
            &exit_clean
        ));
        assert!(!termination_warrants_restart(
            LspRestartPolicy::Never,
            &signaled
        ));
        assert!(termination_warrants_restart(
            LspRestartPolicy::Always,
            &exit_clean
        ));
        assert!(!termination_warrants_restart(
            LspRestartPolicy::OnCrash,
            &exit_clean
        ));
        assert!(termination_warrants_restart(
            LspRestartPolicy::OnCrash,
            &exit_nonzero
        ));
        assert!(termination_warrants_restart(
            LspRestartPolicy::OnCrash,
            &signaled
        ));
    }
}
