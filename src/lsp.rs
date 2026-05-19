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
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use crate::async_runtime::{JobId, JobKind, SharedAsyncRuntime};
use crate::process::{
    ProcessEvent, ProcessEventKind, ProcessId, ProcessMode, ProcessSpec, ProcessState,
    RestartPolicy, Termination,
};
use crate::worker::CancellationToken;

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
    /// T M4.5: workspace settings answered to server→client
    /// `workspace/configuration` pull requests. A JSON object;
    /// requested `section`s (dotted paths, e.g. `python.analysis`)
    /// resolve into it, unknown sections answer `null` per spec.
    pub settings: Option<Value>,
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
            settings: None,
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

/// Negotiated LSP position encoding (LSP 3.17 `general.positionEncoding`).
/// The `character` field of every `Position` is counted in these
/// units. pmacs works in UTF-8 byte offsets everywhere internally
/// (rope, cursor, stores, Lua); this drives the one conversion at the
/// request/response boundary so every consumer stays byte-uniform.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Default)]
pub enum PositionEncoding {
    /// `character` counts UTF-8 code units (== bytes). Conversion is
    /// identity — pmacs's native representation. Advertised first so
    /// servers that support it (rust-analyzer, clangd) pick the
    /// zero-cost path.
    Utf8,
    /// `character` counts UTF-16 code units. The LSP spec default
    /// when a server does not negotiate (gopls / pyright / older
    /// servers). Requires per-line code-unit walking.
    #[default]
    Utf16,
}

impl PositionEncoding {
    /// Map a negotiated `general.positionEncoding` string to the
    /// enum. Anything other than `"utf-8"` (including `"utf-16"`,
    /// `"utf-32"` which we do not advertise, or absent/garbage)
    /// resolves to the spec default, UTF-16.
    fn from_negotiated(s: Option<&str>) -> Self {
        match s {
            Some("utf-8") => Self::Utf8,
            _ => Self::Utf16,
        }
    }
}

/// Resolve a `workspace/configuration` item's `section` against the
/// server's `settings`. LSP semantics: a dotted `section`
/// (`"python.analysis"`) walks nested objects; an absent/empty
/// section asks for the whole settings object; a section that does
/// not resolve answers `null` (the spec's "scope not configured"
/// signal — distinct from a configured `null`/`false`).
fn resolve_config_section(settings: &Value, section: Option<&str>) -> Value {
    match section {
        None | Some("") => settings.clone(),
        Some(path) => {
            let mut cur = settings;
            for key in path.split('.') {
                match cur.get(key) {
                    Some(v) => cur = v,
                    None => return Value::Null,
                }
            }
            cur.clone()
        }
    }
}

/// The 0-based `line`-th `\n`-delimited slice of `text`, or `None`
/// when `line` is past EOF. The distinction matters: a genuinely
/// empty line (`Some("")`) is converted to byte 0, but a line we do
/// not have (`None` — e.g. a cross-file definition into a document
/// we never cached) must be left **unconverted** rather than
/// collapsed to 0, so its coordinates are not corrupted. A trailing
/// `\r` (CRLF) stays in the slice; both the pmacs byte offset and the
/// server `character` are line-relative so the `\r` cancels out.
fn nth_line(text: &str, line: u32) -> Option<&str> {
    text.split('\n').nth(line as usize)
}

/// Snap `byte` down to the nearest char boundary of `line` (defensive
/// against a server, in UTF-8 mode, reporting a mid-codepoint offset).
fn floor_boundary(line: &str, mut byte: usize) -> usize {
    if byte >= line.len() {
        return line.len();
    }
    while byte > 0 && !line.is_char_boundary(byte) {
        byte -= 1;
    }
    byte
}

/// Inbound: server `character` (in `enc` units, line-relative) → the
/// pmacs byte offset within `line`. Clamps past-EOL to `line.len()`.
///
/// `pub(crate)` so the semantic-render producer can reuse the exact
/// same encoding conversion for LSP semantic tokens (whose
/// `start`/`length` are *not* byte-rewritten by the absorb path —
/// see [`LspManager::semantic_style_context`]).
pub(crate) fn char_to_byte(line: &str, character: u32, enc: PositionEncoding) -> usize {
    match enc {
        PositionEncoding::Utf8 => floor_boundary(line, character as usize),
        PositionEncoding::Utf16 => {
            let mut units = 0u32;
            for (byte_idx, ch) in line.char_indices() {
                let cu = ch.len_utf16() as u32;
                // If the target unit falls *within* this char (incl. a
                // server pointing mid-surrogate-pair), clamp to the
                // char's start rather than overshooting to the next.
                if character < units + cu {
                    return byte_idx;
                }
                units += cu;
            }
            line.len()
        }
    }
}

/// Outbound: pmacs byte offset within `line` → server `character` in
/// `enc` units. `byte_col` is clamped to the line and snapped to a
/// char boundary first.
fn byte_to_char(line: &str, byte_col: usize, enc: PositionEncoding) -> u32 {
    let byte_col = floor_boundary(line, byte_col);
    match enc {
        PositionEncoding::Utf8 => byte_col as u32,
        PositionEncoding::Utf16 => {
            let mut units = 0u32;
            for (byte_idx, ch) in line.char_indices() {
                if byte_idx >= byte_col {
                    break;
                }
                units += ch.len_utf16() as u32;
            }
            units
        }
    }
}

/// Recursively rewrite every LSP `Position` (`{ line, character }`)
/// in `value` so `character` becomes a pmacs byte offset instead of
/// a count in `enc` units. In LSP the `(line, character)` key pair
/// uniquely identifies a `Position` — no other structure carries
/// both — so a structural walk is exact: `Range`s, `Location`s,
/// `TextEdit`s, diagnostics, hover ranges all nest `Position`s and
/// are converted uniformly. UTF-8 is a no-op fast path (the offsets
/// already match), so the recursion is skipped entirely there.
fn rewrite_positions_to_bytes(value: &mut Value, doc: &str, enc: PositionEncoding) {
    if enc == PositionEncoding::Utf8 {
        return;
    }
    match value {
        Value::Object(map) => {
            if let (Some(line), Some(character)) = (
                map.get("line").and_then(Value::as_u64),
                map.get("character").and_then(Value::as_u64),
            ) && let Some(line_text) = nth_line(doc, line as u32)
            {
                // Line absent from the cached doc (cross-file /
                // not-yet-opened) ⇒ leave the position untouched.
                let byte = char_to_byte(line_text, character as u32, enc);
                map.insert("character".into(), Value::from(byte));
            }
            for v in map.values_mut() {
                rewrite_positions_to_bytes(v, doc, enc);
            }
        }
        Value::Array(arr) => {
            for v in arr.iter_mut() {
                rewrite_positions_to_bytes(v, doc, enc);
            }
        }
        _ => {}
    }
}

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
    /// T M4.5: request ids we sent `$/cancelRequest` for after every
    /// awaiter abandoned them. A server may still answer a cancelled
    /// request (the cancel/response race); a late response whose id
    /// is in this set is dropped silently instead of surfacing a
    /// "response for unknown request id" `ProtocolError`. Mirrors
    /// `mcp.rs`'s `cancelled_rids`.
    cancelled_rids: HashSet<u64>,
    /// Notifications issued before the server reached `Initialized`.
    /// The LSP lifecycle requires `initialize` / `initialized` to
    /// complete before any other notification; a strict server
    /// (clangd) silently discards a pre-init `textDocument/didOpen`,
    /// after which every later request fails with
    /// `-32602 trying to get AST for non-added document`. These are
    /// held in issue order while `Starting` / `Initializing` and
    /// replayed the instant the server reaches `Initialized` (right
    /// after the `initialized` notification is sent).
    deferred_notifications: Vec<(String, Value)>,
    /// T M4.5 Option B: encoding the server negotiated for `Position`
    /// `character` counts. Set from the `initialize` response;
    /// defaults to the LSP spec default (UTF-16) until then.
    position_encoding: PositionEncoding,
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
            cancelled_rids: HashSet::new(),
            deferred_notifications: Vec::new(),
            position_encoding: PositionEncoding::default(),
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
    /// T M4.5: references / declaration / typeDefinition /
    /// implementation. Same Location shape as `definition`, keyed
    /// additionally by kind so the four don't collide.
    locations_store: crate::locations::SharedLocationsStore,
    /// T M4.5: documentSymbol / workspace symbol, scope-keyed.
    symbol_store: crate::symbol::SharedSymbolStore,
    /// T M4.5: textDocument/documentHighlight, per `(server, uri)`.
    document_highlight_store: crate::document_highlight::SharedDocumentHighlightStore,
    /// T M4.12 formatting store. Populated when a
    /// `textDocument/formatting` response lands.
    formatting_store: crate::formatting::SharedFormattingStore,
    /// T M4.5 L2 rename store. Populated when a `textDocument/rename`
    /// response (a `WorkspaceEdit`) lands, keyed by the origin uri.
    rename_store: crate::rename::SharedRenameStore,
    /// T M4.5 prepareRename store. Populated when a
    /// `textDocument/prepareRename` response lands, keyed by `(server,
    /// uri)`.
    prepare_rename_store: crate::prepare_rename::SharedPrepareRenameStore,
    /// T M4.5 L3 code-action store. Populated when a
    /// `textDocument/codeAction` response lands, keyed by `(server,
    /// uri)`.
    code_action_store: crate::code_action::SharedCodeActionStore,
    /// T M4.5 inlay-hint store. Populated when a
    /// `textDocument/inlayHint` response lands, keyed by `(server,
    /// uri)`.
    inlay_hint_store: crate::inlay_hint::SharedInlayHintStore,
    /// T M4.5 semantic-token store. Populated when a
    /// `textDocument/semanticTokens/full` response lands, keyed by
    /// `(server, uri)`.
    semantic_token_store: crate::semantic_tokens::SharedSemanticTokenStore,
    /// Per-server request id → response routing target.
    /// `request_completion` etc. record an entry here; `handle_response`
    /// consumes it to absorb the response into the correct store.
    pending_routes: HashMap<(LspServerId, u64), ResponseRoute>,
    /// T M4.5 async bridge: `(server, request_id)` → the awaiters
    /// parked on this request's async-runtime job(s). Settled in
    /// [`Self::handle_response`] when the response routes, and
    /// drained-cancelled wherever [`Self::pending_routes`] is purged
    /// for a server (restart / exit / forget) so a coroutine never
    /// parks forever on a server that went away.
    pending_external: HashMap<(LspServerId, u64), PendingExternal>,
    /// T M4.5 Option B: latest full text per `(server, uri)`,
    /// mirrored from the `did_open` / `did_change_full` we send. The
    /// only thing that lets the position codec convert between the
    /// server's `character` units and pmacs byte offsets per line.
    /// Dropped on `did_close` and at every server-teardown site.
    documents: HashMap<(LspServerId, String), String>,
    /// Async runtime handle. The bridge between the supervisor
    /// reader-thread response delivery and Lua-side `Handle:await()`
    /// resumption; mirrors [`crate::mcp::McpManager`]'s `runtime`.
    runtime: SharedAsyncRuntime,
    /// T M4.5 task #8: hard ceiling on how long an in-flight request
    /// may park its awaiters before the sweep fails them with a
    /// timeout. Generous by default (real servers can take seconds on
    /// a cold cache); tunable from Lua via `pmacs.lsp.set_request_timeout_ms`.
    request_timeout: Duration,
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
    /// Absorb a `textDocument/rename` `WorkspaceEdit` into
    /// [`crate::rename::RenameStore`] at `(server, origin uri)`.
    Rename { uri: String },
    /// Absorb a `textDocument/prepareRename` response into
    /// [`crate::prepare_rename::PrepareRenameStore`] at `(server,
    /// uri)`.
    PrepareRename { uri: String },
    /// Absorb a `textDocument/codeAction` response into
    /// [`crate::code_action::CodeActionStore`] at `(server, uri)`.
    CodeAction { uri: String },
    /// Absorb a `textDocument/inlayHint` response into
    /// [`crate::inlay_hint::InlayHintStore`] at `(server, uri)`.
    InlayHint { uri: String },
    /// Absorb a `textDocument/semanticTokens/full` (or `/range`)
    /// response into [`crate::semantic_tokens::SemanticTokenStore`]
    /// at `(server, uri)`.
    SemanticTokens { uri: String },
    /// Absorb a `textDocument/semanticTokens/full/delta` response —
    /// spliced against the store's retained raw int stream at
    /// `(server, uri)` — back into that same entry.
    SemanticTokensDelta { uri: String },
    /// Absorb a Location-shaped nav response (references / declaration
    /// / typeDefinition / implementation) into
    /// [`crate::locations::LocationsStore`] at `(server, uri, kind)`.
    Locations {
        /// Document URI.
        uri: String,
        /// Which nav request this answers.
        kind: crate::locations::LocationKind,
    },
    /// Absorb a `textDocument/documentSymbol` response into
    /// [`crate::symbol::SymbolStore`] at `(server, document uri)`.
    DocumentSymbol {
        /// The requested document URI (also the codec doc key, and
        /// the fallback URI for `DocumentSymbol` items which carry
        /// none).
        uri: String,
    },
    /// Absorb a `workspace/symbol` response into
    /// [`crate::symbol::SymbolStore`] at `(server, query)`.
    WorkspaceSymbol {
        /// The query string this answers.
        query: String,
    },
    /// Absorb a `textDocument/documentHighlight` response into
    /// [`crate::document_highlight::DocumentHighlightStore`].
    DocumentHighlight {
        /// Document URI.
        uri: String,
    },
}

impl ResponseRoute {
    /// The document URI this route targets — the key for the position
    /// codec's document/encoding lookup.
    fn uri(&self) -> &str {
        match self {
            ResponseRoute::Completion { uri }
            | ResponseRoute::Hover { uri }
            | ResponseRoute::Signature { uri }
            | ResponseRoute::Definition { uri }
            | ResponseRoute::Formatting { uri }
            | ResponseRoute::Rename { uri }
            | ResponseRoute::PrepareRename { uri }
            | ResponseRoute::CodeAction { uri }
            | ResponseRoute::InlayHint { uri }
            | ResponseRoute::SemanticTokens { uri }
            | ResponseRoute::SemanticTokensDelta { uri }
            | ResponseRoute::Locations { uri, .. }
            | ResponseRoute::DocumentSymbol { uri }
            | ResponseRoute::DocumentHighlight { uri } => uri,
            // workspace/symbol results span arbitrary files we have
            // not cached — no doc to convert against, so the inbound
            // codec must pass coordinates through untouched (same
            // non-destructive rule as cross-file definition).
            ResponseRoute::WorkspaceSymbol { .. } => "",
        }
    }
}

/// One Lua-visible awaiter bound to an in-flight LSP request. Mirrors
/// [`crate::mcp`]'s `Awaiter`: each carries its own
/// [`CancellationToken`] (minted by [`SharedAsyncRuntime::register_external`])
/// so a single handle can be cancelled without disturbing the
/// in-flight wire request or sibling awaiters.
#[derive(Clone, Debug)]
struct Awaiter {
    job_id: JobId,
    /// Per-awaiter cancellation token minted by `register_external`.
    /// Flipped by `Handle:cancel()` or by supersede; the per-tick
    /// [`LspManager::drain_cancelled_externals`] sweep observes it.
    /// Mirrors `mcp.rs`'s `Awaiter::token`.
    token: CancellationToken,
}

/// In-flight `textDocument/*` request bound to one or more
/// async-runtime jobs. When the JSON-RPC response is routed in
/// [`LspManager::handle_response`] the response is absorbed into the
/// typed store (the hybrid model — popup/gutter consumers are
/// untouched) *and* every non-cancelled awaiter is settled via
/// [`SharedAsyncRuntime::complete_external_ok`] (or `_failed` on a
/// server error). Keyed `(server, request_id)` alongside
/// [`LspManager::pending_routes`]. T M4.5 async bridge.
#[derive(Clone, Debug)]
struct PendingExternal {
    /// JSON-RPC method, kept for the `*workers*`/`*lsp*` observability
    /// surface and protocol-error messages.
    method: String,
    /// Live awaiters. Never empty while this entry exists. The first
    /// is the original caller; later entries are siblings.
    awaiters: Vec<Awaiter>,
    /// When the request went on the wire. T M4.5 task #8: the
    /// per-tick sweep fails awaiters whose request has outlived
    /// [`LspManager::request_timeout`], so a wedged-but-alive server
    /// can't park a coroutine forever (the pre-v1.0 `poll_until` had
    /// 500/1000 ms ceilings; this is the async-era equivalent, but a
    /// generous hard ceiling rather than a UX poll budget).
    dispatched_at: Instant,
}

/// Per-server styling context for one URI's semantic tokens,
/// resolved by [`LspManager::semantic_style_context`]. Decouples the
/// semantic-render producer from `LspServerId` (kept opaque) while
/// still giving it everything it needs to turn raw tokens into
/// byte-anchored, named style spans.
#[derive(Clone, Debug)]
pub struct SemanticStyleContext {
    /// The owning server's negotiated position encoding. The
    /// producer converts token `start`/`length` from these units to
    /// pmacs byte offsets (a no-op when it is `Utf8`).
    pub encoding: PositionEncoding,
    /// The server's `semanticTokensProvider.legend`, if advertised.
    /// `None` ⇒ the producer falls back to raw type indices (the
    /// same degradation `lsp.lua`'s summary already tolerates).
    pub legend: Option<crate::semantic_tokens::SemanticTokensLegend>,
}

impl LspManager {
    /// Construct a fresh manager wired to `supervisor` and the
    /// editor's `runtime`. The runtime bridges the supervisor's
    /// reader-thread response delivery to Lua-side `Handle:await()`
    /// resumption (mirrors [`crate::mcp::McpManager::new`]).
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
            pending_external: HashMap::new(),
            documents: HashMap::new(),
            request_timeout: Duration::from_secs(10),
            restart_backoff: Duration::from_millis(500),
            diag_store: crate::diag::make_shared_store(),
            completion_store: crate::completion::make_shared_store(),
            hover_store: crate::hover::make_shared_store(),
            signature_store: crate::signature::make_shared_store(),
            definition_store: crate::definition::make_shared_store(),
            locations_store: crate::locations::make_shared_store(),
            symbol_store: crate::symbol::make_shared_store(),
            document_highlight_store: crate::document_highlight::make_shared_store(),
            formatting_store: crate::formatting::make_shared_store(),
            rename_store: crate::rename::make_shared_store(),
            prepare_rename_store: crate::prepare_rename::make_shared_store(),
            code_action_store: crate::code_action::make_shared_store(),
            inlay_hint_store: crate::inlay_hint::make_shared_store(),
            semantic_token_store: crate::semantic_tokens::make_shared_store(),
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

    /// Shared locations store — references / declaration /
    /// typeDefinition / implementation (T M4.5).
    #[must_use]
    pub fn locations_store(&self) -> crate::locations::SharedLocationsStore {
        self.locations_store.clone()
    }

    /// Shared symbol store — documentSymbol / workspace symbol (T M4.5).
    #[must_use]
    pub fn symbol_store(&self) -> crate::symbol::SharedSymbolStore {
        self.symbol_store.clone()
    }

    /// Shared documentHighlight store (T M4.5).
    #[must_use]
    pub fn document_highlight_store(
        &self,
    ) -> crate::document_highlight::SharedDocumentHighlightStore {
        self.document_highlight_store.clone()
    }

    /// Shared formatting store (T M4.12).
    #[must_use]
    pub fn formatting_store(&self) -> crate::formatting::SharedFormattingStore {
        self.formatting_store.clone()
    }

    /// Shared rename / `WorkspaceEdit` store (T M4.5 L2).
    #[must_use]
    pub fn rename_store(&self) -> crate::rename::SharedRenameStore {
        self.rename_store.clone()
    }

    /// Shared prepareRename store (T M4.5).
    #[must_use]
    pub fn prepare_rename_store(&self) -> crate::prepare_rename::SharedPrepareRenameStore {
        self.prepare_rename_store.clone()
    }

    /// Shared code-action store (T M4.5 L3).
    #[must_use]
    pub fn code_action_store(&self) -> crate::code_action::SharedCodeActionStore {
        self.code_action_store.clone()
    }

    /// Shared inlay-hint store (T M4.5).
    #[must_use]
    pub fn inlay_hint_store(&self) -> crate::inlay_hint::SharedInlayHintStore {
        self.inlay_hint_store.clone()
    }

    /// Shared semantic-token store (T M4.5).
    #[must_use]
    pub fn semantic_token_store(&self) -> crate::semantic_tokens::SharedSemanticTokenStore {
        self.semantic_token_store.clone()
    }

    /// Styling inputs for `uri`'s semantic tokens: the owning
    /// server's negotiated [`PositionEncoding`] and its
    /// `semanticTokensProvider` legend (if the server advertised
    /// one). `None` when no attached server has tokens for `uri`.
    ///
    /// Step 0 of the semantic-frontend producer arc established why
    /// this is needed: `SemanticToken` `start`/`length` are LSP
    /// encoding units (UTF-16 for clangd's default) and — unlike
    /// inlay hints — are *not* byte-rewritten by the absorb path's
    /// `inbound_converted`, because the relative-encoded `data`
    /// array carries no `Position`-shaped object for the structural
    /// walk to find. The producer therefore converts them itself and
    /// needs the per-server encoding plus the legend to name token
    /// types. The server resolved here is the same one
    /// [`crate::semantic_tokens::SemanticTokenStore::for_uri`]
    /// returns (lowest id), so a producer's token read and this
    /// context read agree on the source.
    #[must_use]
    pub fn semantic_style_context(&self, uri: &str) -> Option<SemanticStyleContext> {
        let server_key = {
            let store = self.semantic_token_store.lock().ok()?;
            store.for_uri(uri).map(|(s, _)| s.to_owned())?
        };
        let sid = self
            .clients
            .keys()
            .copied()
            .find(|id| id.raw().to_string() == server_key)?;
        let legend = self
            .capabilities(sid)
            .and_then(crate::semantic_tokens::SemanticTokensLegend::from_capabilities);
        Some(SemanticStyleContext {
            encoding: self.position_encoding(sid),
            legend,
        })
    }

    /// Test-only: register a synthetic, already-`Initialized` client
    /// (no child process) with the given `initialize` capabilities
    /// and negotiated encoding; returns its id. Lets producer tests
    /// drive the LSP-styling path — where `semantic_style_context`
    /// must resolve a real client's legend + encoding — without a
    /// live server. This is exactly the post-handshake client state
    /// minus the process; nothing here can reach the wire.
    #[cfg(test)]
    pub(crate) fn insert_initialized_test_client(
        &mut self,
        capabilities: Value,
        encoding: PositionEncoding,
    ) -> LspServerId {
        let id = LspServerId::next();
        let mut client = LspClient::new(LspServerSpec::new("test", "test", "true"));
        client.state = LspClientState::Initialized {
            capabilities,
            server_info: None,
            initialized_at: Instant::now(),
        };
        client.position_encoding = encoding;
        self.clients.insert(id, client);
        id
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
        // T M4.5 task #7: the new generation has a fresh request-id
        // space; stale cancelled ids can never collide, so reset to
        // keep the set from growing across restarts.
        client.cancelled_rids.clear();
        // Drop notifications buffered against the dead generation:
        // they reference its (now-gone) document state, and the
        // editor's reattach path issues fresh `did_open`s after the
        // new generation finishes initializing.
        client.deferred_notifications.clear();
        // T M4.7: drop any pending response routes for this server.
        // Their request ids belong to the previous generation; the
        // new server starts request id numbering fresh.
        self.pending_routes.retain(|(sid, _), _| *sid != id);
        // T M4.5: the previous generation's in-flight awaiters will
        // never get a response (new process, fresh id space) — wake
        // them cancelled before the restart.
        self.drain_external_cancelled(id);
        // T M4.5 Option B: drop cached docs; the fresh server gets a
        // new `did_open` from the editor's reattach path.
        self.documents.retain(|(s, _), _| *s != id);
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
        // Disallowed after Stopped/Crashed since stdin is gone.
        if matches!(
            client.state,
            LspClientState::Stopped { .. } | LspClientState::Crashed { .. }
        ) {
            return Err(format!(
                "server {id} is not running (state: {})",
                state_label(&client.state)
            ));
        }
        // Before the `initialize` / `initialized` handshake completes
        // the only notification the spec permits is `initialized`
        // itself — and that one is sent directly by `handle_response`,
        // never through here. Everything else issued this early
        // (`textDocument/didOpen` from the editor's attach hook is the
        // common one) is buffered and replayed in order once the
        // server reaches `Initialized`; sending it now would be
        // discarded by a strict server, breaking every later request
        // with "non-added document".
        if matches!(
            client.state,
            LspClientState::Starting | LspClientState::Initializing { .. }
        ) {
            client.deferred_notifications.push((method, params));
            return Ok(());
        }
        let body = make_notification(&method, params);
        send_frame_to(&self.supervisor, client, &body)?;
        Ok(())
    }

    /// Replay, in issue order, the notifications buffered by
    /// [`Self::send_notification`] while `sid` was still initializing.
    /// Called once, from the `initialize`-response handler, right
    /// after the `initialized` notification is sent.
    fn flush_deferred_notifications(&mut self, sid: LspServerId) {
        let deferred = self
            .clients
            .get_mut(&sid)
            .map(|c| std::mem::take(&mut c.deferred_notifications))
            .unwrap_or_default();
        if !deferred.is_empty()
            && let Some(client) = self.clients.get(&sid)
        {
            for (method, params) in deferred {
                let body = make_notification(&method, params);
                let _ = send_frame_to(&self.supervisor, client, &body);
            }
        }
    }

    /// The position encoding `sid` negotiated, or the spec default
    /// (UTF-16) if the server is unknown / not yet initialized.
    fn position_encoding(&self, sid: LspServerId) -> PositionEncoding {
        self.clients
            .get(&sid)
            .map_or(PositionEncoding::default(), |c| c.position_encoding)
    }

    /// Outbound: build the LSP `Position` for `(line, byte_col)`,
    /// converting the pmacs byte offset into the server's negotiated
    /// `character` units. If the document is not cached (no `did_open`
    /// seen yet) the byte offset is sent as-is — correct for ASCII /
    /// the UTF-8 fast path and the same behaviour pmacs had before
    /// Option B, so the fallback never regresses the common case.
    fn outbound_position(&self, sid: LspServerId, uri: &str, line: u32, byte_col: u32) -> Value {
        let enc = self.position_encoding(sid);
        let character = match self
            .documents
            .get(&(sid, uri.to_owned()))
            .and_then(|doc| nth_line(doc, line))
        {
            Some(line_text) => byte_to_char(line_text, byte_col as usize, enc),
            // No cached doc, or cursor line not in it ⇒ send the byte
            // offset as-is (correct for ASCII / the UTF-8 path and the
            // pre-Option-B behaviour).
            None => byte_col,
        };
        json!({ "line": line, "character": character })
    }

    /// Inbound: clone `value` and rewrite every nested `Position` so
    /// `character` is a pmacs byte offset. No cached document (or the
    /// UTF-8 fast path) ⇒ returned unchanged.
    fn inbound_converted(&self, sid: LspServerId, uri: &str, value: &Value) -> Value {
        let enc = self.position_encoding(sid);
        let mut owned = value.clone();
        if let Some(doc) = self.documents.get(&(sid, uri.to_owned())) {
            rewrite_positions_to_bytes(&mut owned, doc, enc);
        }
        owned
    }

    /// Register an async-runtime awaiter for the just-sent request
    /// `req_id` and return its [`JobId`] — the value the Lua surface
    /// wraps in a `pmacs.workers` Handle. Called only after
    /// [`Self::send_request`] succeeded, so the wire request is live
    /// and a response (or a teardown drain) will settle it; no
    /// rollback path is needed here (unlike `mcp.rs`, where the
    /// register precedes the wire send). T M4.5 async bridge.
    ///
    /// T M4.5 task #7: the awaiter is registered with a supersede key
    /// `lsp:{method}:{sid}:{uri}` — stable across calls for the same
    /// (server, method, document). A newer request for the same thing
    /// flips the prior job's [`CancellationToken`] through the async
    /// runtime's supersede map; the next [`Self::drain_cancelled_externals`]
    /// sweep settles that awaiter cancelled and `$/cancelRequest`s the
    /// in-flight wire request. This is what keeps keystroke-driven
    /// completion from piling up N in-flight requests on the server.
    fn register_awaiter(
        &mut self,
        sid: LspServerId,
        req_id: u64,
        method: &str,
        uri: &str,
    ) -> JobId {
        let supersede = format!("lsp:{method}:{}:{uri}", sid.raw());
        let (job_id, token) = self
            .runtime
            .register_external(JobKind::LspRequest, Some(&supersede));
        self.pending_external.insert(
            (sid, req_id),
            PendingExternal {
                method: method.to_owned(),
                awaiters: vec![Awaiter { job_id, token }],
                dispatched_at: Instant::now(),
            },
        );
        job_id
    }

    /// T M4.5 task #8: override the per-request timeout (default 10s).
    /// Exposed to Lua as `pmacs.lsp.set_request_timeout_ms` and used
    /// by the await-path tests to force fast timeouts.
    pub fn set_request_timeout(&mut self, timeout: Duration) {
        self.request_timeout = timeout;
    }

    /// T M4.5 async bridge: settle every awaiter for `sid` as
    /// cancelled and drop its `pending_external` entries. Called
    /// wherever [`Self::pending_routes`] is purged for a server
    /// (restart generation flip / terminal exit / forget) so a
    /// coroutine parked on a request whose server went away wakes
    /// with `{ tag = "cancelled" }` instead of hanging forever.
    /// Mirrors `mcp.rs`'s drain-on-exit. Idempotent: a second call
    /// finds no entries (and `complete_external_cancelled` is itself
    /// idempotent against double-completion).
    fn drain_external_cancelled(&mut self, sid: LspServerId) {
        let keys: Vec<(LspServerId, u64)> = self
            .pending_external
            .keys()
            .filter(|(s, _)| *s == sid)
            .copied()
            .collect();
        for k in keys {
            if let Some(p) = self.pending_external.remove(&k) {
                for a in &p.awaiters {
                    self.runtime.complete_external_cancelled(a.job_id);
                }
            }
        }
    }

    /// T M4.5 task #7: per-tick per-awaiter cancellation sweep for
    /// `sid`. An awaiter whose [`CancellationToken`] was flipped —
    /// either by `Handle:cancel()` or by being superseded by a newer
    /// same-key request (see [`Self::register_awaiter`]) — is removed
    /// and settled cancelled; the in-flight wire request continues if
    /// sibling awaiters still want it. When the *last* awaiter of a
    /// request abandons it, the entry and its route are dropped, the
    /// rid is recorded in `cancelled_rids` so a late response is
    /// dropped silently, and `$/cancelRequest` is sent best-effort so
    /// the server can stop working. Mirrors `mcp.rs`'s
    /// `drain_cancelled_externals` (LSP has no resource cache, so the
    /// cache-state plumbing is omitted).
    ///
    /// T M4.5 task #8: the same sweep also fails any awaiter whose
    /// request has outlived [`Self::request_timeout`] — a server that
    /// stays alive but never answers a particular id would otherwise
    /// park its coroutine forever. Timed-out entries take the same
    /// abandon path as fully-cancelled ones (`$/cancelRequest` +
    /// `cancelled_rids`), but settle `failed` rather than `cancelled`.
    fn drain_cancelled_externals(&mut self, sid: LspServerId) {
        let mut cancelled_jobs: Vec<JobId> = Vec::new();
        let mut timed_out_jobs: Vec<(JobId, String)> = Vec::new();
        let mut abandoned_rids: Vec<u64> = Vec::new();
        // Captured into locals so the `retain` closure doesn't have to
        // borrow `self` (it already borrows `self.pending_external`).
        let now = Instant::now();
        let timeout = self.request_timeout;
        let timeout_ms = timeout.as_millis();
        self.pending_external.retain(|(s, rid), p| {
            if *s != sid {
                return true;
            }
            let mut still: Vec<Awaiter> = Vec::with_capacity(p.awaiters.len());
            for a in p.awaiters.drain(..) {
                if a.token.is_cancelled() {
                    cancelled_jobs.push(a.job_id);
                } else {
                    still.push(a);
                }
            }
            // T M4.5 task #8: any awaiter that survived the cancel
            // pass but whose request has outlived the timeout fails
            // now — the server is alive but not answering this id.
            if !still.is_empty() && now.duration_since(p.dispatched_at) >= timeout {
                for a in still.drain(..) {
                    timed_out_jobs.push((a.job_id, p.method.clone()));
                }
            }
            p.awaiters = still;
            if p.awaiters.is_empty() {
                abandoned_rids.push(*rid);
                false
            } else {
                true
            }
        });
        for job_id in cancelled_jobs {
            self.runtime.complete_external_cancelled(job_id);
        }
        for (job_id, method) in timed_out_jobs {
            self.runtime.complete_external_failed(
                job_id,
                format!("LSP {method}: request timed out after {timeout_ms}ms"),
            );
        }
        for rid in abandoned_rids {
            self.pending_routes.remove(&(sid, rid));
            if let Some(client) = self.clients.get_mut(&sid) {
                client.pending.remove(&rid);
                client.cancelled_rids.insert(rid);
            }
            self.send_cancel_request(sid, rid);
        }
    }

    /// Send `$/cancelRequest { id }` to `sid`, best-effort. A server
    /// that is not accepting writes (stopped / crashed) is skipped by
    /// [`Self::send_notification`]'s state guard; the `Err` is
    /// intentionally ignored. T M4.5 task #7.
    fn send_cancel_request(&mut self, sid: LspServerId, rid: u64) {
        let _ = self.send_notification(sid, "$/cancelRequest", json!({ "id": rid }));
    }

    /// Send `textDocument/completion` for `uri` at `(line, col)`.
    /// The response is absorbed into the completion store at
    /// `(sid, uri)` (popup consumers untouched) and also settles the
    /// returned awaiter. The same `LspEventKind::Response` event is
    /// still emitted for raw observers. Returns the async-runtime
    /// [`JobId`] the response will settle (`type JobId = u64`, so the
    /// signature is unchanged for existing Rust callers — only the
    /// value's meaning moved from JSON-RPC id to job id, and no
    /// caller consumes it). T M4.5 async bridge.
    pub fn request_completion(
        &mut self,
        sid: LspServerId,
        uri: impl Into<String>,
        line: u32,
        col: u32,
    ) -> Result<JobId, String> {
        let uri = uri.into();
        let params = json!({
            "textDocument": { "uri": uri.clone() },
            "position": self.outbound_position(sid, &uri, line, col)
        });
        let req_id = self.send_request(sid, "textDocument/completion", params)?;
        let job_id = self.register_awaiter(sid, req_id, "textDocument/completion", &uri);
        self.pending_routes
            .insert((sid, req_id), ResponseRoute::Completion { uri });
        Ok(job_id)
    }

    /// Send `textDocument/hover` for `uri` at `(line, col)`. Returns
    /// the async-runtime [`JobId`] the response will settle.
    pub fn request_hover(
        &mut self,
        sid: LspServerId,
        uri: impl Into<String>,
        line: u32,
        col: u32,
    ) -> Result<JobId, String> {
        let uri = uri.into();
        let params = json!({
            "textDocument": { "uri": uri.clone() },
            "position": self.outbound_position(sid, &uri, line, col)
        });
        let req_id = self.send_request(sid, "textDocument/hover", params)?;
        let job_id = self.register_awaiter(sid, req_id, "textDocument/hover", &uri);
        self.pending_routes
            .insert((sid, req_id), ResponseRoute::Hover { uri });
        Ok(job_id)
    }

    /// Send `textDocument/signatureHelp` for `uri` at `(line, col)`.
    /// Returns the async-runtime [`JobId`] the response will settle.
    pub fn request_signature_help(
        &mut self,
        sid: LspServerId,
        uri: impl Into<String>,
        line: u32,
        col: u32,
    ) -> Result<JobId, String> {
        let uri = uri.into();
        let params = json!({
            "textDocument": { "uri": uri.clone() },
            "position": self.outbound_position(sid, &uri, line, col)
        });
        let req_id = self.send_request(sid, "textDocument/signatureHelp", params)?;
        let job_id = self.register_awaiter(sid, req_id, "textDocument/signatureHelp", &uri);
        self.pending_routes
            .insert((sid, req_id), ResponseRoute::Signature { uri });
        Ok(job_id)
    }

    /// Send `textDocument/definition` for `uri` at `(line, col)`. The
    /// response is absorbed into the definition store at `(sid, uri)`.
    /// Returns the async-runtime [`JobId`] the response will settle.
    pub fn request_definition(
        &mut self,
        sid: LspServerId,
        uri: impl Into<String>,
        line: u32,
        col: u32,
    ) -> Result<JobId, String> {
        let uri = uri.into();
        let params = json!({
            "textDocument": { "uri": uri.clone() },
            "position": self.outbound_position(sid, &uri, line, col)
        });
        let req_id = self.send_request(sid, "textDocument/definition", params)?;
        let job_id = self.register_awaiter(sid, req_id, "textDocument/definition", &uri);
        self.pending_routes
            .insert((sid, req_id), ResponseRoute::Definition { uri });
        Ok(job_id)
    }

    /// Shared body for the Location-shaped nav requests. `extra` is
    /// merged into the params object (only `references` uses it, for
    /// `context.includeDeclaration`). The supersede key derives from
    /// the kind's distinct method, so the four requests don't cancel
    /// one another. Returns the async-runtime [`JobId`].
    fn request_locations(
        &mut self,
        sid: LspServerId,
        uri: impl Into<String>,
        line: u32,
        col: u32,
        kind: crate::locations::LocationKind,
        extra: Option<Value>,
    ) -> Result<JobId, String> {
        let uri = uri.into();
        let mut params = json!({
            "textDocument": { "uri": uri.clone() },
            "position": self.outbound_position(sid, &uri, line, col)
        });
        if let Some(Value::Object(ex)) = extra
            && let Some(obj) = params.as_object_mut()
        {
            for (k, v) in ex {
                obj.insert(k, v);
            }
        }
        let method = kind.method();
        let req_id = self.send_request(sid, method, params)?;
        let job_id = self.register_awaiter(sid, req_id, method, &uri);
        self.pending_routes
            .insert((sid, req_id), ResponseRoute::Locations { uri, kind });
        Ok(job_id)
    }

    /// Send `textDocument/references` (with `includeDeclaration`).
    /// Returns the async-runtime [`JobId`].
    pub fn request_references(
        &mut self,
        sid: LspServerId,
        uri: impl Into<String>,
        line: u32,
        col: u32,
    ) -> Result<JobId, String> {
        self.request_locations(
            sid,
            uri,
            line,
            col,
            crate::locations::LocationKind::References,
            Some(json!({ "context": { "includeDeclaration": true } })),
        )
    }

    /// Send `textDocument/declaration`. Returns the [`JobId`].
    pub fn request_declaration(
        &mut self,
        sid: LspServerId,
        uri: impl Into<String>,
        line: u32,
        col: u32,
    ) -> Result<JobId, String> {
        self.request_locations(
            sid,
            uri,
            line,
            col,
            crate::locations::LocationKind::Declaration,
            None,
        )
    }

    /// Send `textDocument/typeDefinition`. Returns the [`JobId`].
    pub fn request_type_definition(
        &mut self,
        sid: LspServerId,
        uri: impl Into<String>,
        line: u32,
        col: u32,
    ) -> Result<JobId, String> {
        self.request_locations(
            sid,
            uri,
            line,
            col,
            crate::locations::LocationKind::TypeDefinition,
            None,
        )
    }

    /// Send `textDocument/implementation`. Returns the [`JobId`].
    pub fn request_implementation(
        &mut self,
        sid: LspServerId,
        uri: impl Into<String>,
        line: u32,
        col: u32,
    ) -> Result<JobId, String> {
        self.request_locations(
            sid,
            uri,
            line,
            col,
            crate::locations::LocationKind::Implementation,
            None,
        )
    }

    /// Send `textDocument/documentSymbol` (no position — whole doc).
    /// Returns the async-runtime [`JobId`].
    pub fn request_document_symbol(
        &mut self,
        sid: LspServerId,
        uri: impl Into<String>,
    ) -> Result<JobId, String> {
        let uri = uri.into();
        let params = json!({ "textDocument": { "uri": uri.clone() } });
        let method = "textDocument/documentSymbol";
        let req_id = self.send_request(sid, method, params)?;
        let job_id = self.register_awaiter(sid, req_id, method, &uri);
        self.pending_routes
            .insert((sid, req_id), ResponseRoute::DocumentSymbol { uri });
        Ok(job_id)
    }

    /// Send `workspace/symbol` for `query`. Returns the [`JobId`].
    pub fn request_workspace_symbol(
        &mut self,
        sid: LspServerId,
        query: impl Into<String>,
    ) -> Result<JobId, String> {
        let query = query.into();
        let params = json!({ "query": query.clone() });
        let method = "workspace/symbol";
        let req_id = self.send_request(sid, method, params)?;
        // The query stands in for the doc URI in the supersede key:
        // a newer query for the same string supersedes the prior.
        let job_id = self.register_awaiter(sid, req_id, method, &query);
        self.pending_routes
            .insert((sid, req_id), ResponseRoute::WorkspaceSymbol { query });
        Ok(job_id)
    }

    /// Send `textDocument/documentHighlight` at `(line, col)`.
    /// Returns the [`JobId`].
    pub fn request_document_highlight(
        &mut self,
        sid: LspServerId,
        uri: impl Into<String>,
        line: u32,
        col: u32,
    ) -> Result<JobId, String> {
        let uri = uri.into();
        let params = json!({
            "textDocument": { "uri": uri.clone() },
            "position": self.outbound_position(sid, &uri, line, col)
        });
        let method = "textDocument/documentHighlight";
        let req_id = self.send_request(sid, method, params)?;
        let job_id = self.register_awaiter(sid, req_id, method, &uri);
        self.pending_routes
            .insert((sid, req_id), ResponseRoute::DocumentHighlight { uri });
        Ok(job_id)
    }

    /// Send `textDocument/formatting` for `uri` with `tab_size` /
    /// `insert_spaces` formatting options. The response is absorbed
    /// into the formatting store at `(sid, uri)`. Returns the
    /// async-runtime [`JobId`] the response will settle.
    pub fn request_formatting(
        &mut self,
        sid: LspServerId,
        uri: impl Into<String>,
        tab_size: u32,
        insert_spaces: bool,
    ) -> Result<JobId, String> {
        let uri = uri.into();
        let params = json!({
            "textDocument": { "uri": uri.clone() },
            "options": {
                "tabSize": tab_size,
                "insertSpaces": insert_spaces,
            }
        });
        let req_id = self.send_request(sid, "textDocument/formatting", params)?;
        let job_id = self.register_awaiter(sid, req_id, "textDocument/formatting", &uri);
        self.pending_routes
            .insert((sid, req_id), ResponseRoute::Formatting { uri });
        Ok(job_id)
    }

    /// Send `textDocument/rename` for the symbol at `(line, col)` in
    /// `uri`, requesting `new_name`. The response is a `WorkspaceEdit`
    /// (possibly multi-file); it is absorbed into the rename store
    /// keyed by the *origin* `uri`. Returns the async-runtime
    /// [`JobId`] the response will settle.
    pub fn request_rename(
        &mut self,
        sid: LspServerId,
        uri: impl Into<String>,
        line: u32,
        col: u32,
        new_name: impl Into<String>,
    ) -> Result<JobId, String> {
        let uri = uri.into();
        let params = json!({
            "textDocument": { "uri": uri.clone() },
            "position": { "line": line, "character": col },
            "newName": new_name.into(),
        });
        let req_id = self.send_request(sid, "textDocument/rename", params)?;
        let job_id = self.register_awaiter(sid, req_id, "textDocument/rename", &uri);
        self.pending_routes
            .insert((sid, req_id), ResponseRoute::Rename { uri });
        Ok(job_id)
    }

    /// Send `textDocument/prepareRename` for the symbol at `(line,
    /// col)` in `uri`. The response (renameable? + extent +
    /// placeholder) is absorbed into the prepareRename store at
    /// `(sid, uri)`. Returns the async-runtime [`JobId`] the response
    /// will settle.
    pub fn request_prepare_rename(
        &mut self,
        sid: LspServerId,
        uri: impl Into<String>,
        line: u32,
        col: u32,
    ) -> Result<JobId, String> {
        let uri = uri.into();
        let params = json!({
            "textDocument": { "uri": uri.clone() },
            "position": { "line": line, "character": col },
        });
        let req_id = self.send_request(sid, "textDocument/prepareRename", params)?;
        let job_id = self.register_awaiter(sid, req_id, "textDocument/prepareRename", &uri);
        self.pending_routes
            .insert((sid, req_id), ResponseRoute::PrepareRename { uri });
        Ok(job_id)
    }

    /// Send `textDocument/codeAction` over `[start, end]` in `uri`.
    /// `context.diagnostics` is passed through verbatim (callers feed
    /// the overlapping diagnostics so quick-fixes resolve); an empty
    /// slice is a valid "no diagnostics" context. The response is
    /// absorbed into the code-action store at `(sid, uri)`. Returns
    /// the async-runtime [`JobId`] the response will settle.
    #[allow(clippy::too_many_arguments)]
    pub fn request_code_action(
        &mut self,
        sid: LspServerId,
        uri: impl Into<String>,
        start_line: u32,
        start_col: u32,
        end_line: u32,
        end_col: u32,
        diagnostics: &[Value],
    ) -> Result<JobId, String> {
        let uri = uri.into();
        let params = json!({
            "textDocument": { "uri": uri.clone() },
            "range": {
                "start": { "line": start_line, "character": start_col },
                "end":   { "line": end_line,   "character": end_col   },
            },
            "context": { "diagnostics": diagnostics },
        });
        let req_id = self.send_request(sid, "textDocument/codeAction", params)?;
        let job_id = self.register_awaiter(sid, req_id, "textDocument/codeAction", &uri);
        self.pending_routes
            .insert((sid, req_id), ResponseRoute::CodeAction { uri });
        Ok(job_id)
    }

    /// Send `textDocument/inlayHint` over `[start, end]` in `uri`
    /// (the visible/whole-buffer range the caller wants annotated).
    /// The response is absorbed into the inlay-hint store at `(sid,
    /// uri)`. Returns the async-runtime [`JobId`] the response will
    /// settle.
    #[allow(clippy::too_many_arguments)]
    pub fn request_inlay_hint(
        &mut self,
        sid: LspServerId,
        uri: impl Into<String>,
        start_line: u32,
        start_col: u32,
        end_line: u32,
        end_col: u32,
    ) -> Result<JobId, String> {
        let uri = uri.into();
        let params = json!({
            "textDocument": { "uri": uri.clone() },
            "range": {
                "start": { "line": start_line, "character": start_col },
                "end":   { "line": end_line,   "character": end_col   },
            },
        });
        let req_id = self.send_request(sid, "textDocument/inlayHint", params)?;
        let job_id = self.register_awaiter(sid, req_id, "textDocument/inlayHint", &uri);
        self.pending_routes
            .insert((sid, req_id), ResponseRoute::InlayHint { uri });
        Ok(job_id)
    }

    /// Send `textDocument/semanticTokens/full` for `uri`. The response
    /// (the relative-encoded `data` array) is decoded and absorbed
    /// into the semantic-token store at `(sid, uri)`. Returns the
    /// async-runtime [`JobId`] the response will settle.
    pub fn request_semantic_tokens(
        &mut self,
        sid: LspServerId,
        uri: impl Into<String>,
    ) -> Result<JobId, String> {
        let uri = uri.into();
        let params = json!({ "textDocument": { "uri": uri.clone() } });
        let req_id = self.send_request(sid, "textDocument/semanticTokens/full", params)?;
        let job_id = self.register_awaiter(sid, req_id, "textDocument/semanticTokens/full", &uri);
        self.pending_routes
            .insert((sid, req_id), ResponseRoute::SemanticTokens { uri });
        Ok(job_id)
    }

    /// Send `textDocument/semanticTokens/range` for the `[start, end]`
    /// slice of `uri` (the visible viewport, for large files). The
    /// response shape is identical to `/full` and shares the same
    /// store entry / route.
    #[allow(clippy::too_many_arguments)]
    pub fn request_semantic_tokens_range(
        &mut self,
        sid: LspServerId,
        uri: impl Into<String>,
        start_line: u32,
        start_col: u32,
        end_line: u32,
        end_col: u32,
    ) -> Result<JobId, String> {
        let uri = uri.into();
        let params = json!({
            "textDocument": { "uri": uri.clone() },
            "range": {
                "start": { "line": start_line, "character": start_col },
                "end":   { "line": end_line,   "character": end_col   },
            },
        });
        let req_id = self.send_request(sid, "textDocument/semanticTokens/range", params)?;
        let job_id = self.register_awaiter(sid, req_id, "textDocument/semanticTokens/range", &uri);
        self.pending_routes
            .insert((sid, req_id), ResponseRoute::SemanticTokens { uri });
        Ok(job_id)
    }

    /// Send `textDocument/semanticTokens/full/delta` for `uri`,
    /// passing the `previous_result_id` the last full/delta response
    /// carried. The response (a `SemanticTokensDelta`, or a full
    /// `SemanticTokens` if the server declined a delta) is spliced
    /// against the store's retained raw int stream.
    pub fn request_semantic_tokens_delta(
        &mut self,
        sid: LspServerId,
        uri: impl Into<String>,
        previous_result_id: impl Into<String>,
    ) -> Result<JobId, String> {
        let uri = uri.into();
        let params = json!({
            "textDocument": { "uri": uri.clone() },
            "previousResultId": previous_result_id.into(),
        });
        let req_id = self.send_request(sid, "textDocument/semanticTokens/full/delta", params)?;
        let job_id =
            self.register_awaiter(sid, req_id, "textDocument/semanticTokens/full/delta", &uri);
        self.pending_routes
            .insert((sid, req_id), ResponseRoute::SemanticTokensDelta { uri });
        Ok(job_id)
    }

    /// Send `workspace/executeCommand`. No response route is
    /// registered: the command result is usually `null` and the real
    /// effect arrives as a server→client `workspace/applyEdit` (the
    /// Lua event pump applies it). The returned awaiter still settles
    /// when the command response lands so callers can sequence work
    /// after it. Returns the async-runtime [`JobId`].
    pub fn request_execute_command(
        &mut self,
        sid: LspServerId,
        command: impl Into<String>,
        arguments: &[Value],
    ) -> Result<JobId, String> {
        let command = command.into();
        let params = json!({ "command": command, "arguments": arguments });
        let req_id = self.send_request(sid, "workspace/executeCommand", params)?;
        // Awaiter only — `absorb_routed_response` has no CodeAction-
        // result store and the effect is delivered out of band.
        Ok(self.register_awaiter(sid, req_id, "workspace/executeCommand", &command))
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
            // T M4.5 task #7: after this tick's responses are absorbed
            // (above), reap any awaiter cancelled or superseded since
            // the last tick.
            self.drain_cancelled_externals(sid);
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
        // T M4.5: the server is gone (clean stop or crash) and will
        // never answer its in-flight requests. Wake every awaiter
        // cancelled now — not at the eventual restart/forget, which
        // may never come under `LspRestartPolicy::Never`.
        self.drain_external_cancelled(sid);
        self.documents.retain(|(s, _), _| *s != sid);
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
            // T M4.5 task #7: the cancel/response race. We sent
            // `$/cancelRequest` for this id after every awaiter
            // abandoned it; a server may answer anyway. By definition
            // no awaiter is listening, so drop it silently rather
            // than emitting a `ProtocolError` for an expected outcome.
            if client.cancelled_rids.remove(&rid) {
                return;
            }
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
            // T M4.5 Option B: honour the server's negotiated
            // `general.positionEncoding`. Absent ⇒ LSP spec default
            // (UTF-16). We advertised `["utf-8","utf-16"]`, so a
            // 3.17 server echoes its pick here; pre-3.17 servers omit
            // it and are correctly treated as UTF-16.
            let negotiated = PositionEncoding::from_negotiated(
                caps.get("positionEncoding").and_then(Value::as_str),
            );
            if let Some(client) = self.clients.get_mut(&sid) {
                client.position_encoding = negotiated;
                client.state = LspClientState::Initialized {
                    capabilities: caps.clone(),
                    server_info,
                    initialized_at: now,
                };
            }
            // The handshake is complete: replay every notification
            // buffered while the server was still initializing, after
            // the `initialized` notification that went out above.
            self.flush_deferred_notifications(sid);
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
        // T M4.5 async bridge: settle every awaiter parked on this
        // request. Deliberately independent of the store-absorb guard
        // above — a null result (e.g. "no hover here") is still a
        // successful response and must wake `:await()` with `nil`
        // rather than leave the coroutine parked forever. A server
        // error settles the awaiter as failed so `:await()` raises the
        // structured `{ tag = "failed" }`. The typed store was already
        // populated above when present (hybrid model); popup/gutter
        // consumers are unaffected by this block.
        if let Some(p) = self.pending_external.remove(&(sid, rid)) {
            if let Some(err) = error.as_ref() {
                let msg = format!("LSP {} error {}: {}", p.method, err.code, err.message);
                for a in &p.awaiters {
                    self.runtime.complete_external_failed(a.job_id, msg.clone());
                }
            } else {
                let value = result.clone().unwrap_or(Value::Null);
                for a in &p.awaiters {
                    self.runtime.complete_external_ok(a.job_id, value.clone());
                }
            }
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

    // One arm per `ResponseRoute` variant — grows by a few lines with
    // each new typed store; the dispatch stays a flat match.
    #[allow(clippy::too_many_lines)]
    fn absorb_routed_response(&self, sid: LspServerId, route: &ResponseRoute, result: &Value) {
        // T M4.5 Option B: normalise every `Position` in the response
        // to pmacs byte offsets *before* the typed store parses it, so
        // the completion popup, diagnostics gutter, and lsp.lua all
        // stay byte-uniform with zero per-consumer changes.
        let converted = self.inbound_converted(sid, route.uri(), result);
        let result = &converted;
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
            ResponseRoute::Locations { uri, kind } => {
                // Same Location parser as definition; only the store
                // key carries the kind discriminator.
                let resp = crate::definition::DefinitionResponse::from_lsp_value(result);
                let key = crate::locations::LocationsKey::new(server_key, uri.clone(), *kind);
                let mut guard = self
                    .locations_store
                    .lock()
                    .expect("locations store mutex poisoned");
                guard.set(key, resp);
            }
            ResponseRoute::DocumentSymbol { uri } => {
                // DocumentSymbol items carry no URI — they belong to
                // the requested document; pass it as the fallback.
                let resp = crate::symbol::SymbolResponse::from_lsp_value(result, uri);
                let key = crate::symbol::SymbolKey::document(server_key, uri.clone());
                let mut guard = self
                    .symbol_store
                    .lock()
                    .expect("symbol store mutex poisoned");
                guard.set(key, resp);
            }
            ResponseRoute::WorkspaceSymbol { query } => {
                let resp = crate::symbol::SymbolResponse::from_lsp_value(result, "");
                let key = crate::symbol::SymbolKey::workspace(server_key, query.clone());
                let mut guard = self
                    .symbol_store
                    .lock()
                    .expect("symbol store mutex poisoned");
                guard.set(key, resp);
            }
            ResponseRoute::DocumentHighlight { uri } => {
                let resp =
                    crate::document_highlight::DocumentHighlightResponse::from_lsp_value(result);
                let key =
                    crate::document_highlight::DocumentHighlightKey::new(server_key, uri.clone());
                let mut guard = self
                    .document_highlight_store
                    .lock()
                    .expect("document highlight store mutex poisoned");
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
            ResponseRoute::Rename { uri } => {
                let resp = crate::rename::WorkspaceEditResponse::from_lsp_value(result);
                let key = crate::rename::RenameKey::new(server_key, uri.clone());
                let mut guard = self
                    .rename_store
                    .lock()
                    .expect("rename store mutex poisoned");
                guard.set(key, resp);
            }
            ResponseRoute::PrepareRename { uri } => {
                let resp = crate::prepare_rename::PrepareRenameResponse::from_lsp_value(result);
                let key = crate::prepare_rename::PrepareRenameKey::new(server_key, uri.clone());
                let mut guard = self
                    .prepare_rename_store
                    .lock()
                    .expect("prepare rename store mutex poisoned");
                guard.set(key, resp);
            }
            ResponseRoute::CodeAction { uri } => {
                let resp = crate::code_action::CodeActionResponse::from_lsp_value(result);
                let key = crate::code_action::CodeActionKey::new(server_key, uri.clone());
                let mut guard = self
                    .code_action_store
                    .lock()
                    .expect("code action store mutex poisoned");
                guard.set(key, resp);
            }
            ResponseRoute::InlayHint { uri } => {
                let resp = crate::inlay_hint::InlayHintResponse::from_lsp_value(result);
                let key = crate::inlay_hint::InlayHintKey::new(server_key, uri.clone());
                let mut guard = self
                    .inlay_hint_store
                    .lock()
                    .expect("inlay hint store mutex poisoned");
                guard.set(key, resp);
            }
            ResponseRoute::SemanticTokens { uri } => {
                let resp = crate::semantic_tokens::SemanticTokensResponse::from_lsp_value(result);
                let key = crate::semantic_tokens::SemanticTokenKey::new(server_key, uri.clone());
                let mut guard = self
                    .semantic_token_store
                    .lock()
                    .expect("semantic token store mutex poisoned");
                guard.set(key, resp);
            }
            ResponseRoute::SemanticTokensDelta { uri } => {
                let key = crate::semantic_tokens::SemanticTokenKey::new(server_key, uri.clone());
                let mut guard = self
                    .semantic_token_store
                    .lock()
                    .expect("semantic token store mutex poisoned");
                // Splice against whatever raw stream the previous
                // full/delta left here; empty if none (the server
                // should then have answered full, which apply_delta
                // detects and parses).
                let prev_raw = guard.get(&key).map(|r| r.raw.clone()).unwrap_or_default();
                let resp =
                    crate::semantic_tokens::SemanticTokensResponse::apply_delta(&prev_raw, result);
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
        // T M4.5: answer `workspace/configuration` ourselves — it's a
        // protocol-level pull (gopls/pyright/clangd issue it during
        // startup and degrade without a reply), not something to defer
        // to a Lua consumer. One array element per requested item;
        // each `section` resolves against the server's `settings`,
        // unknown sections answer `null` per spec.
        if method == "workspace/configuration" {
            let settings = self
                .clients
                .get(&sid)
                .and_then(|c| c.spec.settings.clone())
                .unwrap_or(Value::Null);
            let answers: Vec<Value> = params
                .get("items")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .map(|item| {
                            resolve_config_section(
                                &settings,
                                item.get("section").and_then(Value::as_str),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            let _ = self.send_response(sid, idv, Ok(Value::Array(answers)));
            return;
        }
        // Everything else: expose the request to the consumer and let
        // it reply via `send_response`. The LSP spec tolerates
        // delayed responses; unrecognised requests simply linger.
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
            self.absorb_publish_diagnostics(sid, &params);
        }
        self.push_event(sid, now, LspEventKind::Notification { method, params });
    }

    /// Parse `params` as a `PublishDiagnosticsParams` payload and
    /// update [`Self::diag_store`]. Malformed payloads (missing uri
    /// or non-array diagnostics) are ignored --- they're a
    /// server-side bug, not a fatal protocol error.
    fn absorb_publish_diagnostics(&self, sid: LspServerId, params: &Value) {
        let Some(uri) = params.get("uri").and_then(Value::as_str).map(str::to_owned) else {
            return;
        };
        // T M4.5 Option B: byte-normalise diagnostic ranges before the
        // store parses them, so the gutter renders correct spans on
        // non-ASCII lines.
        let converted = self.inbound_converted(sid, &uri, params);
        let Some(arr) = converted.get("diagnostics").and_then(Value::as_array) else {
            return;
        };
        let parsed: Vec<crate::diag::Diagnostic> = arr
            .iter()
            .filter_map(crate::diag::Diagnostic::from_lsp_value)
            .collect();
        let mut guard = self.diag_store.lock().expect("diag store mutex poisoned");
        guard.set(uri, parsed);
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
        // T M4.5: belt-and-braces — on_exit already drained on the
        // terminal transition; this catches any awaiter registered
        // between exit and forget. Idempotent.
        self.drain_external_cancelled(sid);
        self.documents.retain(|(s, _), _| *s != sid);
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
        let uri = uri.into();
        let text = text.into();
        // T M4.5 Option B: mirror the document so the position codec
        // can convert per-line between the server's `character` units
        // and pmacs byte offsets.
        self.documents.insert((sid, uri.clone()), text.clone());
        let params = json!({
            "textDocument": {
                "uri": uri,
                "languageId": language_id,
                "version": version,
                "text": text,
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
        let uri = uri.into();
        let text = text.into();
        self.documents.insert((sid, uri.clone()), text.clone());
        let params = json!({
            "textDocument": {
                "uri": uri,
                "version": version,
            },
            "contentChanges": [{
                "text": text,
            }],
        });
        self.send_notification(sid, "textDocument/didChange", params)
    }

    /// Send `workspace/didChangeWatchedFiles` to `sid`. `changes` is
    /// the already-shaped `FileEvent[]` array (`[{ uri, type }]`,
    /// type 1=created / 2=changed / 3=deleted) the Lua file-watch
    /// module builds. T M4.5.
    pub fn did_change_watched_files(
        &mut self,
        sid: LspServerId,
        changes: &Value,
    ) -> Result<(), String> {
        self.send_notification(
            sid,
            "workspace/didChangeWatchedFiles",
            json!({ "changes": changes }),
        )
    }

    /// Convenience: send `textDocument/didClose` to `sid`.
    pub fn did_close(&mut self, sid: LspServerId, uri: impl Into<String>) -> Result<(), String> {
        let uri = uri.into();
        self.documents.remove(&(sid, uri.clone()));
        let params = json!({
            "textDocument": { "uri": uri },
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
        // T M4.5 Option B: LSP 3.17 position-encoding negotiation.
        // UTF-8 first (identity for pmacs's byte offsets); UTF-16 is
        // the mandatory fallback every server understands.
        "general": {
            "positionEncodings": ["utf-8", "utf-16"],
        },
        "workspace": {
            // T M4.5 L3: pmacs applies server→client `workspace/
            // applyEdit` requests via the Lua WorkspaceEdit applier
            // (surfaced as a request event, answered with
            // `{ applied }`). executeCommand-driven code actions
            // depend on this.
            "applyEdit": true,
            "executeCommand": { "dynamicRegistration": false },
            // T M4.5: pmacs answers server→client `workspace/configuration`
            // pull requests from the per-server `settings` (see
            // `handle_request`). gopls / pyright / clangd all pull
            // config this way and degrade without it.
            "configuration": true,
            "workspaceFolders": true,
            "didChangeConfiguration": { "dynamicRegistration": false },
            // T M4.5 — file watching. `dynamicRegistration: true` is
            // mandatory: clangd / rust-analyzer / gopls only ever
            // register `workspace/didChangeWatchedFiles` dynamically
            // (via `client/registerCapability`); without it the
            // server never asks us to watch and the feature is dead.
            // The Lua server-request pump handles the registration
            // and runs the snapshot-diff watcher.
            "didChangeWatchedFiles": { "dynamicRegistration": true },
            // T M4.5 — let servers tell us cached inlay hints /
            // semantic tokens are stale via a server→client
            // `workspace/inlayHint/refresh` /
            // `workspace/semanticTokens/refresh` request. The Lua
            // server-request pump answers each and re-pulls the
            // affected documents into the matching store.
            "inlayHint": { "refreshSupport": true },
            "semanticTokens": { "refreshSupport": true },
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
            // T M4.5: client-side rename. `prepareSupport: true` —
            // the rename flow does a `textDocument/prepareRename`
            // round-trip first *when the server advertises
            // `renameProvider.prepareProvider`* (it gates the prompt
            // and pre-fills the placeholder); otherwise it sends
            // `textDocument/rename` straight from the cursor.
            // `prepareSupportDefaultBehavior: 1` (Identifier) tells
            // the server we can handle the `{ defaultBehavior }`
            // shape (compute the word range ourselves).
            "rename": {
                "dynamicRegistration": false,
                "prepareSupport": true,
                "prepareSupportDefaultBehavior": 1,
            },
            // T M4.5 L3: code actions. `codeActionLiteralSupport`
            // tells servers we accept the richer `CodeAction` shape
            // (title/kind/edit/command), not just bare `Command`s.
            "codeAction": {
                "dynamicRegistration": false,
                "codeActionLiteralSupport": {
                    "codeActionKind": {
                        "valueSet": [
                            "quickfix", "refactor", "refactor.extract",
                            "refactor.inline", "refactor.rewrite",
                            "source", "source.organizeImports",
                        ],
                    },
                },
            },
            // T M4.5 inlay hints. No `resolveSupport` — pmacs
            // requests full hints (label/tooltip already populated),
            // not lazily-resolved stubs. `refreshSupport` is left
            // false (default) so servers don't send
            // `workspace/inlayHint/refresh`; on-demand re-query is
            // the v1 model.
            "inlayHint": { "dynamicRegistration": false },
            // T M4.5 semantic tokens. We support `/full`, the
            // `/full/delta` follow-up, and the `/range` viewport
            // request. `formats: ["relative"]` is the only encoding
            // LSP defines; the tokenTypes/tokenModifiers lists are
            // the LSP-standard legend the client understands — the
            // server intersects its legend with these and reports the
            // agreed legend back via `semanticTokensProvider.legend`.
            "semanticTokens": {
                "dynamicRegistration": false,
                "requests": { "full": { "delta": true }, "range": true },
                "formats": ["relative"],
                "tokenTypes": [
                    "namespace", "type", "class", "enum", "interface",
                    "struct", "typeParameter", "parameter", "variable",
                    "property", "enumMember", "event", "function",
                    "method", "macro", "keyword", "modifier", "comment",
                    "string", "number", "regexp", "operator", "decorator"
                ],
                "tokenModifiers": [
                    "declaration", "definition", "readonly", "static",
                    "deprecated", "abstract", "async", "modification",
                    "documentation", "defaultLibrary"
                ],
            },
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

    // ---- T M4.5 Option B: position codec -------------------------------

    #[test]
    fn position_encoding_negotiation_defaults_to_utf16() {
        assert_eq!(
            PositionEncoding::from_negotiated(Some("utf-8")),
            PositionEncoding::Utf8
        );
        assert_eq!(
            PositionEncoding::from_negotiated(Some("utf-16")),
            PositionEncoding::Utf16
        );
        // Absent or unrecognised ⇒ LSP spec default (UTF-16).
        assert_eq!(
            PositionEncoding::from_negotiated(None),
            PositionEncoding::Utf16
        );
        assert_eq!(
            PositionEncoding::from_negotiated(Some("utf-32")),
            PositionEncoding::Utf16
        );
    }

    #[test]
    fn codec_utf8_is_identity_on_char_boundaries() {
        // "é=x": é = 2 bytes (U+00E9), '=' , 'x'  → len 4.
        let line = "é=x";
        for &b in &[0usize, 2, 3, 4] {
            assert_eq!(byte_to_char(line, b, PositionEncoding::Utf8), b as u32);
            assert_eq!(char_to_byte(line, b as u32, PositionEncoding::Utf8), b);
        }
        // A mid-codepoint byte snaps down to the char boundary.
        assert_eq!(byte_to_char(line, 1, PositionEncoding::Utf8), 0);
        assert_eq!(char_to_byte(line, 1, PositionEncoding::Utf8), 0);
    }

    #[test]
    fn codec_utf16_round_trips_through_non_ascii() {
        // "é=x": UTF-16 units é=0 '='=1 'x'=2 ; bytes é=0..2 '='=2 'x'=3.
        let line = "é=x";
        // byte → utf16
        assert_eq!(byte_to_char(line, 0, PositionEncoding::Utf16), 0);
        assert_eq!(byte_to_char(line, 2, PositionEncoding::Utf16), 1); // '='
        assert_eq!(byte_to_char(line, 3, PositionEncoding::Utf16), 2); // 'x'
        // utf16 → byte
        assert_eq!(char_to_byte(line, 0, PositionEncoding::Utf16), 0);
        assert_eq!(char_to_byte(line, 1, PositionEncoding::Utf16), 2);
        assert_eq!(char_to_byte(line, 2, PositionEncoding::Utf16), 3);
        // Round-trip both directions over every boundary.
        for &byte in &[0usize, 2, 3] {
            let c = byte_to_char(line, byte, PositionEncoding::Utf16);
            assert_eq!(char_to_byte(line, c, PositionEncoding::Utf16), byte);
        }
    }

    #[test]
    fn codec_utf16_handles_astral_surrogate_pair() {
        // "🦀x": U+1F980 is 4 UTF-8 bytes and a UTF-16 surrogate pair
        // (2 code units). 'x' is byte 4, UTF-16 unit 2.
        let line = "🦀x";
        assert_eq!(byte_to_char(line, 4, PositionEncoding::Utf16), 2);
        assert_eq!(char_to_byte(line, 2, PositionEncoding::Utf16), 4);
        // A server pointing inside the surrogate pair (unit 1) clamps
        // to the start of the crab rather than splitting the codepoint.
        assert_eq!(char_to_byte(line, 1, PositionEncoding::Utf16), 0);
    }

    #[test]
    fn nth_line_picks_the_right_slice() {
        let doc = "abc\nré\n\nx";
        assert_eq!(nth_line(doc, 0), Some("abc"));
        assert_eq!(nth_line(doc, 1), Some("ré"));
        assert_eq!(nth_line(doc, 2), Some("")); // genuinely empty line
        assert_eq!(nth_line(doc, 3), Some("x"));
        assert_eq!(nth_line(doc, 9), None); // past EOF ⇒ leave unconverted
    }

    #[test]
    fn rewrite_positions_recurses_ranges_and_skips_utf8() {
        let doc = "é=x"; // line 0
        // A definition-shaped Location[]: Range start/end are Positions.
        let mut v = json!([{
            "uri": "file:///x",
            "range": {
                "start": { "line": 0, "character": 1 },  // utf16 '=' → byte 2
                "end":   { "line": 0, "character": 2 }    // utf16 'x' → byte 3
            }
        }]);
        rewrite_positions_to_bytes(&mut v, doc, PositionEncoding::Utf16);
        let r = &v[0]["range"];
        assert_eq!(r["start"]["character"], json!(2));
        assert_eq!(r["end"]["character"], json!(3));
        assert_eq!(r["start"]["line"], json!(0)); // line untouched

        // UTF-8 fast path: structurally identical input is unchanged.
        let mut v2 = json!({ "line": 0, "character": 1 });
        rewrite_positions_to_bytes(&mut v2, doc, PositionEncoding::Utf8);
        assert_eq!(v2["character"], json!(1));
    }

    // ---- T M4.5: workspace/configuration section resolution --------------

    #[test]
    fn resolve_config_section_semantics() {
        let s = json!({
            "python": { "analysis": { "typeCheckingMode": "basic" } },
            "x": 1,
            "nullable": null,
        });
        // Dotted path walks nested objects.
        assert_eq!(
            resolve_config_section(&s, Some("python.analysis.typeCheckingMode")),
            json!("basic")
        );
        assert_eq!(
            resolve_config_section(&s, Some("python.analysis")),
            json!({ "typeCheckingMode": "basic" })
        );
        assert_eq!(resolve_config_section(&s, Some("x")), json!(1));
        // A configured `null` is returned as-is (distinct from unknown).
        assert_eq!(resolve_config_section(&s, Some("nullable")), Value::Null);
        // Unknown section ⇒ null (the spec's "not configured" signal).
        assert_eq!(
            resolve_config_section(&s, Some("python.missing")),
            Value::Null
        );
        assert_eq!(resolve_config_section(&s, Some("nope")), Value::Null);
        // Absent / empty section ⇒ the whole settings object.
        assert_eq!(resolve_config_section(&s, None), s);
        assert_eq!(resolve_config_section(&s, Some("")), s);
    }
}
