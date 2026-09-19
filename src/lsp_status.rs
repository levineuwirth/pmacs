// lsp_status.rs --- T M4.8 LSP status surface: state machine + log.

//! Per-server LSP status the modeline and `*lsp*` buffer read from.
//!
//! Per spec §M4.8: "state (initializing, ready, indexing, degraded,
//! crashed) visible at all times" and "last error retrievable via a
//! binding". This module folds the raw [`crate::lsp::LspEvent`]
//! stream into a stable, queryable [`LspStatus`].
//!
//! # State machine
//!
//! ```text
//!     Initializing ─── Initialized event ──→ Ready
//!         │                                   │
//!         │                       $/progress  │  Indexing
//!         │           begin on an INDEXING ─→│
//!         │           token (see below)       │ end ←──┐
//!         │                                   ↑        │
//!         │                                   └────────┘
//!         │
//!         ├── ProtocolError / a server-failure  ─→ Degraded { reason }
//!         │       ↑   response error (see below)   │
//!         │       └─ further failures stay         │ next Initialized
//!         │          degraded                      │ flips back to Ready
//!         │
//!         ├── Crashed event → Crashed { reason }
//!         └── ShuttingDown / Stopped → Stopped
//! ```
//!
//! # Busy is not a state (E7b.1)
//!
//! Every `$/progress` cycle is tracked as in-flight work beside `kind`
//! ([`LspStatus::busy`]), and only the tokens in [`INDEXING_TOKENS`]
//! move `kind` to `Indexing`: those are rust-analyzer's cache priming
//! and its startup family, the work during which the server cannot
//! answer for the crate. Everything else --- a flycheck after a save,
//! a root rescan, any progress from a server this list does not know
//! --- leaves `kind` alone and shows as a short suffix on the label
//! (`ready·check`), so the modeline says `ready` whenever the server
//! is. Before this the tracker flipped to `Indexing` on every begin
//! and back on every end, and on this repository rust-analyzer reports
//! progress most of the time, so `ready` showed only in the gaps. A
//! `Degraded` kind is disturbed by neither; an indexing cycle that is
//! still in flight when the degraded window closes is what the kind
//! returns to.
//!
//! # Degraded means the server failed (C7b fix round 1)
//!
//! An error response moves the kind by its code and by nothing else.
//! The owner's ruling partitions JSON-RPC's codes three ways:
//!
//! * **The server failed**: `-32603` `InternalError` and the server
//!   error range `-32099..=-32000` (`ServerNotInitialized`,
//!   `UnknownErrorCode` and a server's own codes in it), together with
//!   transport loss (`ProtocolError`, `Crashed`). These set
//!   `last_error`, enter `Degraded` for [`DEGRADED_STICKY`] and log in
//!   `*lsp*`.
//! * **pmacs sent something wrong**: `-32600` `InvalidRequest`,
//!   `-32601` `MethodNotFound`, `-32602` `InvalidParams`. The server
//!   answered; it is not unwell. These log in `*lsp*` and leave the
//!   kind and `last_error` alone, and the Lua drain reports each one
//!   through `pmacs.error` into `*errors*` with the method and code.
//!   Before this, `M-x lsp.rename` on a blank line --- rust-analyzer
//!   answers `prepareRename` there with `-32602` --- read `degraded`
//!   on the modeline for fifteen seconds.
//! * **The request is moot**: `-32800` `RequestCancelled` and
//!   `-32801` `ContentModified` are counted and otherwise silent
//!   (E6d.2).
//!
//! Any other code (`-32700` `ParseError`, `-32802` `ServerCancelled`,
//! `-32803` `RequestFailed`, a code a server coins outside the range
//! such as rust-analyzer's `-32900` for a formatter it could not run,
//! an unknown one) logs in `*lsp*` and leaves the kind alone: nothing
//! outside the first class is a server failure.
//!
//! # Why a separate module
//!
//! [`crate::lsp::LspManager`] already tracks the low-level lifecycle
//! ([`crate::lsp::LspClientState`]). The status surface is a *higher*
//! abstraction — what the modeline says — and keeping it apart lets
//! the modeline consume one stable enum without re-deriving "ready
//! vs indexing" on every render.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::lsp::{LspClientState, LspError, LspEvent, LspEventKind, LspServerId};

// ---------------------------------------------------------------------------
// Status kind
// ---------------------------------------------------------------------------

/// High-level status for one LSP server, consumed by the modeline /
/// `*lsp*` buffer / Lua surface.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LspStatusKind {
    /// Process spawned, `initialize` request in flight (or just sent).
    Initializing,
    /// `initialize` response received; ready to serve requests.
    Ready,
    /// Server is doing background work (rust-analyzer indexing,
    /// pyright analysing the project, etc.). Surfaces the current
    /// `$/progress` `title` and `percentage` (if reported).
    Indexing {
        /// Human-readable progress title (e.g. "indexing").
        title: String,
        /// 0..=100 percent if reported, otherwise `None`.
        percentage: Option<u32>,
    },
    /// Something went wrong but the server is still running. The
    /// modeline shows the last error message; the next clean
    /// transition (`Initialized`, `$/progress` `end`) flips it back.
    Degraded {
        /// One-line summary of why the server is degraded.
        reason: String,
    },
    /// Server died unexpectedly. Stays sticky until a successful
    /// restart re-emits `Initialized`.
    Crashed {
        /// Display-formatted exit/signal reason.
        reason: String,
    },
    /// Server was asked to shut down or has cleanly exited. Terminal.
    Stopped,
}

impl LspStatusKind {
    /// Short modeline label (≤ 9 characters). `*lsp*` and the modeline
    /// share this so the user sees one consistent name.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Initializing => "init",
            Self::Ready => "ready",
            Self::Indexing { .. } => "idx",
            Self::Degraded { .. } => "degraded",
            Self::Crashed { .. } => "crashed",
            Self::Stopped => "stopped",
        }
    }

    /// Stable kind tag used by the Lua surface and the `*lsp*` buffer
    /// (e.g. `"ready"`, `"indexing"`, `"degraded"`). Distinct from
    /// [`Self::label`] so the Lua side gets the long form.
    #[must_use]
    pub fn tag(&self) -> &'static str {
        match self {
            Self::Initializing => "initializing",
            Self::Ready => "ready",
            Self::Indexing { .. } => "indexing",
            Self::Degraded { .. } => "degraded",
            Self::Crashed { .. } => "crashed",
            Self::Stopped => "stopped",
        }
    }
}

// ---------------------------------------------------------------------------
// Recent messages
// ---------------------------------------------------------------------------

/// Default ring-buffer capacity for [`LspStatus::recent_messages`].
/// Sized to hold the chatter from one `initialize` round-trip plus a
/// handful of follow-on notifications without recycling.
pub const DEFAULT_MESSAGE_CAPACITY: usize = 64;

/// One entry in the per-server recent-message ring. Distinct from
/// [`crate::lsp::LspEvent`] because the status log is *display-shaped*:
/// it carries a short summary plus an optional detail line, ready to
/// drop into the `*lsp*` buffer.
#[derive(Clone, Debug)]
pub struct LspStatusMessage {
    /// Monotonic timestamp.
    pub at: Instant,
    /// Severity / channel of the line: one of `"info"`, `"warn"`,
    /// `"error"`, `"stderr"`. Used for colouring and for filtering
    /// the log.
    pub channel: &'static str,
    /// One-line summary suitable for the `*lsp*` buffer's first
    /// column.
    pub summary: String,
    /// Optional follow-on detail (e.g. an error's `data` field).
    pub detail: Option<String>,
}

// ---------------------------------------------------------------------------
// Per-server status
// ---------------------------------------------------------------------------

/// Per-server LSP status. Derived from the [`LspEvent`] stream by
/// [`LspStatusTracker`].
#[derive(Clone, Debug)]
pub struct LspStatus {
    /// Current state machine bucket.
    pub kind: LspStatusKind,
    /// Most recent error observed for this server, if any. Sticky
    /// until cleared via [`LspStatus::clear_last_error`] or another
    /// error replaces it. Surfaces through `pmacs.lsp.last_error`.
    pub last_error: Option<LspStatusError>,
    /// When [`Self::kind`] last changed.
    pub last_state_change: Instant,
    /// Cumulative restart count for this server (mirrored from the
    /// LSP layer). The status surface uses this for the
    /// `Degraded` heuristic.
    pub restarts: u32,
    /// `serverInfo` field from the most recent `initialize`
    /// response, if reported.
    pub server_info: Option<LspServerInfo>,
    /// Ring buffer of recent log lines.
    pub recent_messages: Vec<LspStatusMessage>,
    /// Capacity of [`Self::recent_messages`].
    pub message_capacity: usize,
    /// Responses answered with a retry code (E6d.2): requests the
    /// document moved out from under, counted rather than logged, so
    /// the `*lsp*` surface can say how often the server was asked
    /// about text that was already gone.
    pub retry_responses: u64,
    /// Every `$/progress` cycle begun and not yet ended, oldest first
    /// (E7b.1). The indexing ones decide `kind`; the others are what
    /// [`Self::busy`] reports.
    pub progress: Vec<ProgressInFlight>,
    /// The title of the newest in-flight progress cycle that is NOT
    /// indexing --- a flycheck, typically --- or `None` when the
    /// server reports no such work. Recomputed on every progress
    /// notification so a per-frame read costs nothing.
    pub busy: Option<String>,
}

/// One `$/progress` cycle the server has begun and not ended.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProgressInFlight {
    /// The `token` of the cycle, as a string (`rustAnalyzer/…`, or a
    /// number spelled out).
    pub token: String,
    /// The `title` from `begin`, or the placeholder when it had none.
    pub title: String,
    /// Whether the token is one of [`INDEXING_TOKENS`].
    pub indexing: bool,
    /// The last reported percentage, if any.
    pub percentage: Option<u32>,
}

/// The `$/progress` tokens that mean the server cannot yet answer for
/// the workspace, enumerated from rust-analyzer's source at the version
/// on the machine E7b.1 was written on (`rust-analyzer 1 (9074e9b4c6
/// 2026-09-06)`, `crates/rust-analyzer/src/{main_loop,reload}.rs`):
/// every `report_progress` there names its token `rustAnalyzer/<title>`
/// unless it passes a cancel token, and cache priming passes
/// `rustAnalyzer/cachePriming` under the title `Indexing`. The flycheck
/// cycle, `rust-analyzer/flycheck/<n>` titled after the check command,
/// is deliberately absent: a check does not make the server unready.
/// `rustAnalyzer/Indexing` is kept for the versions that named the
/// priming token after its title.
pub const INDEXING_TOKENS: &[&str] = &[
    "rustAnalyzer/cachePriming",
    "rustAnalyzer/Indexing",
    "rustAnalyzer/Fetching",
    "rustAnalyzer/Roots Scanned",
    "rustAnalyzer/Building CrateGraph",
    "rustAnalyzer/Building compile-time-deps",
    "rustAnalyzer/Loading proc-macros",
];

/// Whether `token` is one of [`INDEXING_TOKENS`].
#[must_use]
pub fn is_indexing_token(token: &str) -> bool {
    INDEXING_TOKENS.contains(&token)
}

impl LspStatus {
    /// Initial status for a freshly spawned server. Starts in
    /// [`LspStatusKind::Initializing`] with no error and an empty log.
    #[must_use]
    pub fn new(now: Instant) -> Self {
        Self {
            kind: LspStatusKind::Initializing,
            last_error: None,
            last_state_change: now,
            restarts: 0,
            server_info: None,
            recent_messages: Vec::with_capacity(DEFAULT_MESSAGE_CAPACITY),
            message_capacity: DEFAULT_MESSAGE_CAPACITY,
            retry_responses: 0,
            progress: Vec::new(),
            busy: None,
        }
    }

    /// The modeline's text for this server: the kind's label, and when
    /// the kind is `Ready` with non-indexing work in flight, that work
    /// as a short suffix --- `ready·check` during a `cargo check`. The
    /// suffix is the title's first word, lowercased, without a leading
    /// `cargo`, at most eight characters; it is never a replacement for
    /// the label, so a reader who wants "is the server ready" reads the
    /// prefix and nothing else.
    #[must_use]
    pub fn modeline_text(&self) -> String {
        let label = self.kind.label();
        match (&self.kind, self.busy.as_deref()) {
            (LspStatusKind::Ready, Some(title)) => format!("{label}·{}", busy_suffix(title)),
            _ => label.to_owned(),
        }
    }

    /// Recompute [`Self::busy`] from [`Self::progress`].
    fn recompute_busy(&mut self) {
        self.busy = self
            .progress
            .iter()
            .rev()
            .find(|p| !p.indexing)
            .map(|p| p.title.clone());
    }

    /// The newest indexing cycle still in flight, if any.
    fn indexing_in_flight(&self) -> Option<&ProgressInFlight> {
        self.progress.iter().rev().find(|p| p.indexing)
    }

    /// Forget every in-flight cycle: the server restarted, crashed or
    /// stopped, and its progress went with it.
    fn clear_progress(&mut self) {
        self.progress.clear();
        self.busy = None;
    }

    /// Replace the kind, bumping `last_state_change` if it changed.
    pub fn set_kind(&mut self, kind: LspStatusKind, now: Instant) {
        if self.kind != kind {
            self.kind = kind;
            self.last_state_change = now;
        }
    }

    /// Append a status-log line, dropping the oldest entry if the
    /// ring is at capacity.
    pub fn push_message(&mut self, m: LspStatusMessage) {
        if self.recent_messages.len() >= self.message_capacity && !self.recent_messages.is_empty() {
            self.recent_messages.remove(0);
        }
        self.recent_messages.push(m);
    }

    /// Drop the sticky last error.
    pub fn clear_last_error(&mut self) {
        self.last_error = None;
    }
}

/// Sticky last-error record. Keeps just enough to repro the failure
/// without dragging the entire JSON-RPC error around.
#[derive(Clone, Debug)]
pub struct LspStatusError {
    /// When the error was observed.
    pub at: Instant,
    /// One of `"protocol"`, `"crash"`, `"response"`. The Lua surface
    /// surfaces this as a tag so callers can filter (e.g. ignore
    /// stderr noise).
    pub source: &'static str,
    /// Display-formatted message.
    pub message: String,
    /// Optional numeric code (JSON-RPC error code for `"response"`,
    /// signal/exit code for `"crash"`, none for `"protocol"`).
    pub code: Option<i64>,
}

impl LspStatusError {
    fn from_lsp_error(at: Instant, e: &LspError) -> Self {
        Self {
            at,
            source: "response",
            message: e.message.clone(),
            code: Some(e.code),
        }
    }
}

/// Subset of LSP `serverInfo` we keep around for the status buffer.
#[derive(Clone, Debug)]
pub struct LspServerInfo {
    /// `name` field.
    pub name: String,
    /// `version` field if present.
    pub version: Option<String>,
}

impl LspServerInfo {
    /// Parse from a `serverInfo` JSON object. Returns `None` for
    /// non-object inputs.
    #[must_use]
    pub fn from_value(v: &Value) -> Option<Self> {
        let name = v.get("name")?.as_str()?.to_owned();
        let version = v.get("version").and_then(Value::as_str).map(str::to_owned);
        Some(Self { name, version })
    }
}

// ---------------------------------------------------------------------------
// Tracker
// ---------------------------------------------------------------------------

/// How long a `Degraded` state stays sticky after the last error.
/// Past this window, an otherwise-`Ready` server clears its degraded
/// flag automatically.
pub const DEGRADED_STICKY: Duration = Duration::from_secs(15);

/// `RequestCancelled` (LSP 3.17 §3.3): the client asked for the
/// request to stop, which every superseded request does.
pub const REQUEST_CANCELLED: i64 = -32800;

/// `ContentModified` (LSP 3.17 §3.3): the document moved under the
/// request and the answer would describe text that is gone. Under
/// typing this is the normal case, not a failure of the server.
pub const CONTENT_MODIFIED: i64 = -32801;

/// Whether an error response is one of the spec's retry codes, which
/// say the request is moot rather than that the server is unwell:
/// neither flips the status to `Degraded`, sets `last_error`, nor
/// lands in the recent messages (E6d.2). The modeline read "degraded"
/// through ordinary typing because every keystroke's `didChange` made
/// rust-analyzer answer the in-flight token request with
/// `ContentModified`, and each such answer re-armed the fifteen-second
/// sticky window.
#[must_use]
pub const fn is_retry_code(code: i64) -> bool {
    matches!(code, REQUEST_CANCELLED | CONTENT_MODIFIED)
}

/// `InternalError` (JSON-RPC 2.0): the server failed on a request it
/// could parse and route.
pub const INTERNAL_ERROR: i64 = -32603;

/// The JSON-RPC server-error range, `-32099..=-32000`: reserved for a
/// server's own failures (`ServerNotInitialized` `-32002`,
/// `UnknownErrorCode` `-32001` and the codes a server defines in it).
pub const SERVER_ERROR_MIN: i64 = -32099;
/// The upper end of the server-error range, inclusive.
pub const SERVER_ERROR_MAX: i64 = -32000;

/// Whether an error response says the server failed --- the one class
/// of response that moves the kind to `Degraded` (the module doc's
/// "Degraded means the server failed"). Everything else is either the
/// client's own mistake, a moot request, or a request the server
/// declined on its merits, and none of those is the server being
/// unwell.
#[must_use]
pub const fn is_server_failure_code(code: i64) -> bool {
    code == INTERNAL_ERROR || (SERVER_ERROR_MIN <= code && code <= SERVER_ERROR_MAX)
}

/// Per-manager status tracker. Holds one [`LspStatus`] per known
/// server and folds events into it.
#[derive(Default)]
pub struct LspStatusTracker {
    by_server: HashMap<LspServerId, LspStatus>,
}

impl LspStatusTracker {
    /// Empty tracker.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Ensure a status entry exists for `sid`. Idempotent. Called by
    /// the LSP manager on every spawn.
    pub fn ensure(&mut self, sid: LspServerId, now: Instant) -> &mut LspStatus {
        self.by_server
            .entry(sid)
            .or_insert_with(|| LspStatus::new(now))
    }

    /// Forget about `sid`. Mirrors [`crate::lsp::LspManager::forget`].
    pub fn forget(&mut self, sid: LspServerId) {
        self.by_server.remove(&sid);
    }

    /// Look up the current status for `sid`.
    #[must_use]
    pub fn get(&self, sid: LspServerId) -> Option<&LspStatus> {
        self.by_server.get(&sid)
    }

    /// All known server ids (for the `*lsp*` buffer).
    pub fn servers(&self) -> impl Iterator<Item = LspServerId> + '_ {
        self.by_server.keys().copied()
    }

    /// Fold one [`LspEvent`] into `sid`'s status. `client_state` is
    /// the LSP layer's *current* state for the server (the one that
    /// would result from the same event sequence) — the tracker uses
    /// it as a tiebreaker (e.g. so a `ProtocolError` while
    /// `Initializing` keeps the modeline saying "init" rather than
    /// "degraded").
    #[allow(
        clippy::too_many_lines,
        reason = "linear dispatch over LspEventKind variants; splitting fragments the per-variant logic"
    )]
    pub fn observe(&mut self, ev: &LspEvent, client_state: Option<&LspClientState>) {
        let st = self.ensure(ev.server, ev.at);
        match &ev.kind {
            LspEventKind::Started { pid } => {
                st.clear_progress();
                st.set_kind(LspStatusKind::Initializing, ev.at);
                st.push_message(LspStatusMessage {
                    at: ev.at,
                    channel: "info",
                    summary: format!("started (pid {pid})"),
                    detail: None,
                });
            }
            LspEventKind::Initialized { capabilities } => {
                let server_info = capabilities
                    .get("serverInfo")
                    .and_then(LspServerInfo::from_value);
                if server_info.is_some() {
                    st.server_info = server_info;
                }
                st.set_kind(LspStatusKind::Ready, ev.at);
                st.push_message(LspStatusMessage {
                    at: ev.at,
                    channel: "info",
                    summary: "initialized".into(),
                    detail: None,
                });
            }
            LspEventKind::Notification { method, params } => {
                if method == "$/progress" {
                    if let Some(update) = parse_progress(params) {
                        apply_progress(st, ev.at, &update);
                    }
                } else if method == "window/logMessage" {
                    let (channel, text) = parse_log_message(params);
                    st.push_message(LspStatusMessage {
                        at: ev.at,
                        channel,
                        summary: text,
                        detail: None,
                    });
                } else if method == "window/showMessage" {
                    // E7c.4: a message the server asks the client to
                    // show reaches `*lsp*` like a log line, marked as
                    // the server's own words; until now it reached
                    // nothing, and rust-analyzer's refusal of the
                    // shipped config (#281) was said on every start to
                    // no one. The Lua drain puts the error and warning
                    // ones on the status line as well.
                    let (channel, text) = parse_log_message(params);
                    st.push_message(LspStatusMessage {
                        at: ev.at,
                        channel,
                        summary: format!("server says: {text}"),
                        detail: None,
                    });
                }
            }
            LspEventKind::Request { method, .. } => {
                st.push_message(LspStatusMessage {
                    at: ev.at,
                    channel: "info",
                    summary: format!("server→client request: {method}"),
                    detail: None,
                });
            }
            LspEventKind::Response { method, error, .. } => {
                if error.as_ref().is_some_and(|err| is_retry_code(err.code)) {
                    st.retry_responses += 1;
                }
                if let Some(err) = error.as_ref().filter(|err| !is_retry_code(err.code)) {
                    // Degraded means the server failed: only a
                    // server-failure code sets `last_error` (which is
                    // what keeps the sticky window armed) and moves the
                    // kind. A client-caused or declined request is
                    // logged here and reported by the Lua drain; the
                    // label stays what the server's health makes it.
                    if is_server_failure_code(err.code) {
                        st.last_error = Some(LspStatusError::from_lsp_error(ev.at, err));
                        st.set_kind(
                            LspStatusKind::Degraded {
                                reason: format!("{method}: {} (code {})", err.message, err.code),
                            },
                            ev.at,
                        );
                    }
                    st.push_message(LspStatusMessage {
                        at: ev.at,
                        channel: "error",
                        summary: format!("response error: {method}"),
                        detail: Some(format!("{} (code {})", err.message, err.code)),
                    });
                }
            }
            LspEventKind::ShuttingDown => {
                st.clear_progress();
                st.set_kind(LspStatusKind::Stopped, ev.at);
                st.push_message(LspStatusMessage {
                    at: ev.at,
                    channel: "info",
                    summary: "shutting down".into(),
                    detail: None,
                });
            }
            LspEventKind::Stopped => {
                st.clear_progress();
                st.set_kind(LspStatusKind::Stopped, ev.at);
                st.push_message(LspStatusMessage {
                    at: ev.at,
                    channel: "info",
                    summary: "stopped".into(),
                    detail: None,
                });
            }
            LspEventKind::Crashed { reason } => {
                st.clear_progress();
                st.last_error = Some(LspStatusError {
                    at: ev.at,
                    source: "crash",
                    message: reason.clone(),
                    code: None,
                });
                st.set_kind(
                    LspStatusKind::Crashed {
                        reason: reason.clone(),
                    },
                    ev.at,
                );
                st.push_message(LspStatusMessage {
                    at: ev.at,
                    channel: "error",
                    summary: "crashed".into(),
                    detail: Some(reason.clone()),
                });
            }
            LspEventKind::Restarting { attempt } => {
                st.restarts = *attempt;
                st.push_message(LspStatusMessage {
                    at: ev.at,
                    channel: "warn",
                    summary: format!("restarting (attempt {attempt})"),
                    detail: None,
                });
            }
            LspEventKind::Stderr(bytes) => {
                let text = String::from_utf8_lossy(bytes).into_owned();
                for line in text.lines() {
                    if line.is_empty() {
                        continue;
                    }
                    st.push_message(LspStatusMessage {
                        at: ev.at,
                        channel: "stderr",
                        summary: line.to_owned(),
                        detail: None,
                    });
                }
            }
            LspEventKind::ProtocolError { message } => {
                st.last_error = Some(LspStatusError {
                    at: ev.at,
                    source: "protocol",
                    message: message.clone(),
                    code: None,
                });
                // While Initializing, a protocol error before the
                // first `Initialized` keeps the modeline saying
                // "init" — the LSP layer will surface a Crashed
                // shortly if the server actually died. Otherwise
                // flip to Degraded.
                let initializing = matches!(
                    client_state,
                    Some(LspClientState::Starting | LspClientState::Initializing { .. })
                );
                if !initializing {
                    st.set_kind(
                        LspStatusKind::Degraded {
                            reason: message.clone(),
                        },
                        ev.at,
                    );
                }
                st.push_message(LspStatusMessage {
                    at: ev.at,
                    channel: "error",
                    summary: "protocol error".into(),
                    detail: Some(message.clone()),
                });
            }
        }
    }

    /// Periodic housekeeping. Releases the sticky `Degraded` state
    /// after [`DEGRADED_STICKY`] has elapsed since the last error,
    /// returning the server to `Ready` provided the LSP layer still
    /// reports it as initialised.
    pub fn tick(
        &mut self,
        now: Instant,
        mut current_state: impl FnMut(LspServerId) -> Option<LspClientState>,
    ) {
        for (sid, st) in &mut self.by_server {
            let LspStatusKind::Degraded { .. } = &st.kind else {
                continue;
            };
            let stale = st
                .last_error
                .as_ref()
                .is_none_or(|e| now.duration_since(e.at) >= DEGRADED_STICKY);
            if !stale {
                continue;
            }
            // Only flip back to Ready if the LSP layer agrees the
            // server is still alive and initialised.
            if matches!(
                current_state(*sid),
                Some(LspClientState::Initialized { .. })
            ) {
                // E7b.1: an indexing cycle that began under the
                // degraded window is what the kind returns to.
                st.kind = match st.indexing_in_flight() {
                    Some(p) => LspStatusKind::Indexing {
                        title: p.title.clone(),
                        percentage: p.percentage,
                    },
                    None => LspStatusKind::Ready,
                };
                st.last_state_change = now;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// $/progress parsing
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct ProgressUpdate {
    token: String,
    kind: ProgressKind,
    title: Option<String>,
    percentage: Option<u32>,
    message: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProgressKind {
    Begin,
    Report,
    End,
}

fn parse_progress(params: &Value) -> Option<ProgressUpdate> {
    // LSP 3.17 §3.15: a token is a string or a number. Spelled out
    // either way so the in-flight table has one key type.
    let token = match params.get("token")? {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    let value = params.get("value")?;
    let kind = match value.get("kind").and_then(Value::as_str)? {
        "begin" => ProgressKind::Begin,
        "report" => ProgressKind::Report,
        "end" => ProgressKind::End,
        _ => return None,
    };
    let title = value
        .get("title")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let percentage = value
        .get("percentage")
        .and_then(Value::as_u64)
        .map(|n| n.min(100) as u32);
    let message = value
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_owned);
    Some(ProgressUpdate {
        token,
        kind,
        title,
        percentage,
        message,
    })
}

fn apply_progress(st: &mut LspStatus, at: Instant, update: &ProgressUpdate) {
    match update.kind {
        ProgressKind::Begin | ProgressKind::Report => {
            // A `report` for a token this tracker never saw begin is
            // taken as its begin: the LSP requires the begin, but a
            // tracker attached late has still to show the work.
            let index = if let Some(i) = st.progress.iter().position(|p| p.token == update.token) {
                i
            } else {
                st.progress.push(ProgressInFlight {
                    token: update.token.clone(),
                    title: update.title.clone().unwrap_or_else(|| "working".into()),
                    indexing: is_indexing_token(&update.token),
                    percentage: None,
                });
                st.progress.len() - 1
            };
            {
                let entry = &mut st.progress[index];
                // Hold onto the title we had if this update doesn't
                // carry one (LSP `report` updates are allowed to drop
                // it).
                if let Some(title) = &update.title {
                    entry.title.clone_from(title);
                }
                if update.percentage.is_some() {
                    entry.percentage = update.percentage;
                }
            }
            let entry = st.progress[index].clone();
            // Only an indexing token moves the kind, and only from the
            // states that mean "serving": a degraded server stays
            // degraded (the sticky window and `tick` own that exit),
            // and a server not yet initialized keeps saying so.
            if entry.indexing
                && matches!(
                    st.kind,
                    LspStatusKind::Ready | LspStatusKind::Indexing { .. }
                )
            {
                st.set_kind(
                    LspStatusKind::Indexing {
                        title: entry.title,
                        percentage: entry.percentage,
                    },
                    at,
                );
            }
            st.recompute_busy();
            if let Some(msg) = &update.message {
                st.push_message(LspStatusMessage {
                    at,
                    channel: "info",
                    summary: format!("progress: {msg}"),
                    detail: None,
                });
            }
        }
        ProgressKind::End => {
            let ended = st
                .progress
                .iter()
                .position(|p| p.token == update.token)
                .map(|i| st.progress.remove(i));
            // End → Ready only when the ended cycle was an indexing one
            // and we were actually indexing, so a flycheck's end and an
            // out-of-band `end` alike leave `Degraded` (and every other
            // kind) where it is. Another indexing cycle still in flight
            // keeps the kind, under that cycle's title.
            if ended.is_some_and(|p| p.indexing)
                && matches!(st.kind, LspStatusKind::Indexing { .. })
            {
                let next = st.indexing_in_flight().cloned();
                st.set_kind(
                    match next {
                        Some(p) => LspStatusKind::Indexing {
                            title: p.title,
                            percentage: p.percentage,
                        },
                        None => LspStatusKind::Ready,
                    },
                    at,
                );
            }
            st.recompute_busy();
            if let Some(msg) = &update.message {
                st.push_message(LspStatusMessage {
                    at,
                    channel: "info",
                    summary: format!("progress done: {msg}"),
                    detail: None,
                });
            }
        }
    }
}

/// The short form of a busy title for the modeline: first word,
/// lowercased, a leading `cargo` dropped, at most eight characters.
fn busy_suffix(title: &str) -> String {
    let lower = title.to_lowercase();
    let rest = lower.strip_prefix("cargo ").unwrap_or(&lower);
    rest.split_whitespace()
        .next()
        .unwrap_or("busy")
        .chars()
        .take(8)
        .collect()
}

fn parse_log_message(params: &Value) -> (&'static str, String) {
    #[allow(
        clippy::match_same_arms,
        reason = "explicit code-4 arm documents the LSP MessageType=Log mapping; the wildcard is also info"
    )]
    let channel = match params.get("type").and_then(Value::as_i64) {
        Some(1) => "error",
        Some(2) => "warn",
        Some(4) => "info",
        // 3 = info, 5 = debug, anything else → info.
        _ => "info",
    };
    let message = params
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    (channel, message)
}

// ---------------------------------------------------------------------------
// `*lsp*` buffer formatting
// ---------------------------------------------------------------------------

/// Render the contents of the `*lsp*` status buffer. One section per
/// server with state, capabilities highlights, and the recent log.
///
/// Caller-supplied `label` resolver lets the formatter print the
/// human label the user gave at spawn time without reaching back
/// into the LSP layer for every server.
#[must_use]
pub fn format_status_buffer(
    tracker: &LspStatusTracker,
    label_for: impl Fn(LspServerId) -> Option<String>,
    capabilities_for: impl Fn(LspServerId) -> Option<Value>,
    now: Instant,
) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let mut servers: Vec<LspServerId> = tracker.servers().collect();
    servers.sort_by_key(|s| s.raw());
    if servers.is_empty() {
        out.push_str("No active LSP servers.\n");
        return out;
    }
    let _ = writeln!(out, "LSP servers: {}", servers.len());
    out.push_str("=================================================\n\n");
    for sid in servers {
        let Some(st) = tracker.get(sid) else { continue };
        let label = label_for(sid).unwrap_or_else(|| "<unnamed>".into());
        let _ = writeln!(
            out,
            "[{label}] id={raw}  state={tag}",
            raw = sid.raw(),
            tag = st.kind.tag()
        );
        if let LspStatusKind::Indexing { title, percentage } = &st.kind {
            match percentage {
                Some(p) => {
                    let _ = writeln!(out, "  indexing: {title} ({p}%)");
                }
                None => {
                    let _ = writeln!(out, "  indexing: {title}");
                }
            }
        }
        if let LspStatusKind::Degraded { reason } | LspStatusKind::Crashed { reason } = &st.kind {
            let _ = writeln!(out, "  reason: {reason}");
        }
        for p in st.progress.iter().filter(|p| !p.indexing) {
            match p.percentage {
                Some(pct) => {
                    let _ = writeln!(out, "  busy: {} ({pct}%)  [{}]", p.title, p.token);
                }
                None => {
                    let _ = writeln!(out, "  busy: {}  [{}]", p.title, p.token);
                }
            }
        }
        if st.restarts > 0 {
            let _ = writeln!(out, "  restarts: {}", st.restarts);
        }
        if let Some(info) = &st.server_info {
            match info.version.as_deref() {
                Some(v) => {
                    let _ = writeln!(out, "  server: {} {}", info.name, v);
                }
                None => {
                    let _ = writeln!(out, "  server: {}", info.name);
                }
            }
        }
        if let Some(err) = &st.last_error {
            let age = now.saturating_duration_since(err.at);
            let code = match err.code {
                Some(c) => format!(" (code {c})"),
                None => String::new(),
            };
            let _ = writeln!(
                out,
                "  last error [{src}]{code}: {msg}  ({secs}s ago)",
                src = err.source,
                msg = err.message,
                secs = age.as_secs()
            );
        }
        if let Some(caps) = capabilities_for(sid) {
            let cap_keys = caps_summary(&caps);
            if !cap_keys.is_empty() {
                out.push_str("  capabilities: ");
                out.push_str(&cap_keys.join(", "));
                out.push('\n');
            }
        }
        if !st.recent_messages.is_empty() {
            out.push_str("  recent:\n");
            let total = st.recent_messages.len();
            let start = total.saturating_sub(8);
            for m in &st.recent_messages[start..] {
                let age = now.saturating_duration_since(m.at);
                let _ = writeln!(
                    out,
                    "    [{ch}] {summary}  ({secs}s ago)",
                    ch = m.channel,
                    summary = m.summary,
                    secs = age.as_secs()
                );
                if let Some(d) = &m.detail {
                    let _ = writeln!(out, "       └─ {d}");
                }
            }
        }
        out.push('\n');
    }
    out
}

/// Pull a short list of "interesting" capability flags from a JSON
/// `ServerCapabilities` object. Used by [`format_status_buffer`].
fn caps_summary(caps: &Value) -> Vec<&'static str> {
    let mut out = Vec::new();
    let has = |k: &str| caps.get(k).is_some();
    if has("textDocumentSync") || has("textDocumentSyncKind") {
        out.push("sync");
    }
    if has("hoverProvider") {
        out.push("hover");
    }
    if has("completionProvider") {
        out.push("completion");
    }
    if has("signatureHelpProvider") || has("signatureHelp") {
        out.push("signature");
    }
    if has("definitionProvider") {
        out.push("definition");
    }
    if has("referencesProvider") {
        out.push("references");
    }
    if has("documentSymbolProvider") {
        out.push("symbols");
    }
    if has("diagnosticProvider") {
        out.push("diagnostics");
    }
    if has("renameProvider") {
        out.push("rename");
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ev(server: LspServerId, kind: LspEventKind, at: Instant) -> LspEvent {
        LspEvent { server, kind, at }
    }

    #[test]
    fn label_and_tag_for_each_kind() {
        for (k, label, tag) in [
            (LspStatusKind::Initializing, "init", "initializing"),
            (LspStatusKind::Ready, "ready", "ready"),
            (
                LspStatusKind::Indexing {
                    title: "x".into(),
                    percentage: Some(10),
                },
                "idx",
                "indexing",
            ),
            (
                LspStatusKind::Degraded { reason: "x".into() },
                "degraded",
                "degraded",
            ),
            (
                LspStatusKind::Crashed { reason: "x".into() },
                "crashed",
                "crashed",
            ),
            (LspStatusKind::Stopped, "stopped", "stopped"),
        ] {
            assert_eq!(k.label(), label);
            assert_eq!(k.tag(), tag);
        }
    }

    #[test]
    fn observe_started_then_initialized_transitions_to_ready() {
        let mut t = LspStatusTracker::new();
        let sid = LspServerId::next();
        let now = Instant::now();
        t.observe(&ev(sid, LspEventKind::Started { pid: 42 }, now), None);
        assert_eq!(t.get(sid).unwrap().kind, LspStatusKind::Initializing);
        t.observe(
            &ev(
                sid,
                LspEventKind::Initialized {
                    capabilities: json!({"serverInfo": {"name": "x", "version": "1"}}),
                },
                now,
            ),
            None,
        );
        assert_eq!(t.get(sid).unwrap().kind, LspStatusKind::Ready);
        assert_eq!(t.get(sid).unwrap().server_info.as_ref().unwrap().name, "x");
    }

    #[test]
    fn protocol_error_during_initializing_does_not_flip_to_degraded() {
        let mut t = LspStatusTracker::new();
        let sid = LspServerId::next();
        let now = Instant::now();
        t.observe(&ev(sid, LspEventKind::Started { pid: 1 }, now), None);
        t.observe(
            &ev(
                sid,
                LspEventKind::ProtocolError {
                    message: "garbage frame".into(),
                },
                now,
            ),
            Some(&LspClientState::Initializing {
                init_request_id: 1,
                started: now,
            }),
        );
        assert_eq!(t.get(sid).unwrap().kind, LspStatusKind::Initializing);
        assert!(t.get(sid).unwrap().last_error.is_some());
    }

    #[test]
    fn protocol_error_after_ready_flips_to_degraded() {
        let mut t = LspStatusTracker::new();
        let sid = LspServerId::next();
        let now = Instant::now();
        t.observe(
            &ev(
                sid,
                LspEventKind::Initialized {
                    capabilities: json!({}),
                },
                now,
            ),
            None,
        );
        t.observe(
            &ev(
                sid,
                LspEventKind::ProtocolError {
                    message: "bad frame".into(),
                },
                now,
            ),
            Some(&LspClientState::Initialized {
                capabilities: json!({}),
                server_info: None,
                initialized_at: now,
            }),
        );
        let st = t.get(sid).unwrap();
        assert!(matches!(st.kind, LspStatusKind::Degraded { .. }));
        assert_eq!(st.last_error.as_ref().unwrap().source, "protocol");
    }

    fn progress(sid: LspServerId, token: &str, value: &Value, at: Instant) -> LspEvent {
        ev(
            sid,
            LspEventKind::Notification {
                method: "$/progress".into(),
                params: json!({ "token": token, "value": value.clone() }),
            },
            at,
        )
    }

    fn ready_tracker(sid: LspServerId, now: Instant) -> LspStatusTracker {
        let mut t = LspStatusTracker::new();
        t.observe(
            &ev(
                sid,
                LspEventKind::Initialized {
                    capabilities: json!({}),
                },
                now,
            ),
            None,
        );
        assert_eq!(t.get(sid).unwrap().kind, LspStatusKind::Ready);
        t
    }

    /// The indexing cycle, under rust-analyzer's own token: begin flips
    /// the kind, a report without a title keeps the title, end flips
    /// it back. Bitten by removing the token from `INDEXING_TOKENS`.
    #[test]
    fn progress_begin_to_end_cycles_indexing() {
        let sid = LspServerId::next();
        let now = Instant::now();
        let mut t = ready_tracker(sid, now);
        let token = "rustAnalyzer/cachePriming";
        t.observe(
            &progress(
                sid,
                token,
                &json!({ "kind": "begin", "title": "Indexing", "percentage": 5 }),
                now,
            ),
            None,
        );
        let kind = t.get(sid).unwrap().kind.clone();
        assert!(matches!(kind, LspStatusKind::Indexing { ref title, .. } if title == "Indexing"));
        assert_eq!(t.get(sid).unwrap().modeline_text(), "idx");
        assert_eq!(
            t.get(sid).unwrap().busy,
            None,
            "indexing is the kind, not busy"
        );
        t.observe(
            &progress(
                sid,
                token,
                &json!({ "kind": "report", "percentage": 50, "message": "3/7 (pmacs)" }),
                now,
            ),
            None,
        );
        if let LspStatusKind::Indexing { percentage, title } = &t.get(sid).unwrap().kind {
            assert_eq!(*percentage, Some(50));
            // Title from begin must persist across report w/o title.
            assert_eq!(title, "Indexing");
        } else {
            panic!("expected Indexing");
        }
        t.observe(&progress(sid, token, &json!({ "kind": "end" }), now), None);
        assert_eq!(t.get(sid).unwrap().kind, LspStatusKind::Ready);
        assert_eq!(t.get(sid).unwrap().modeline_text(), "ready");
    }

    /// E7b.1: a flycheck cycle --- rust-analyzer's token, its title the
    /// check command --- never leaves `Ready`; it is busy, rendered as a
    /// suffix, and gone at the end.
    #[test]
    fn a_flycheck_cycle_leaves_the_kind_ready_and_shows_as_a_suffix() {
        let sid = LspServerId::next();
        let now = Instant::now();
        let mut t = ready_tracker(sid, now);
        let token = "rust-analyzer/flycheck/0";
        t.observe(
            &progress(
                sid,
                token,
                &json!({ "kind": "begin", "title": "cargo check", "cancellable": true }),
                now,
            ),
            None,
        );
        let st = t.get(sid).unwrap();
        assert_eq!(st.kind, LspStatusKind::Ready);
        assert_eq!(st.busy.as_deref(), Some("cargo check"));
        assert_eq!(st.modeline_text(), "ready·check");
        t.observe(
            &progress(
                sid,
                token,
                &json!({ "kind": "report", "message": "pmacs" }),
                now,
            ),
            None,
        );
        let st = t.get(sid).unwrap();
        assert_eq!(st.kind, LspStatusKind::Ready);
        assert_eq!(st.modeline_text(), "ready·check");
        t.observe(&progress(sid, token, &json!({ "kind": "end" }), now), None);
        let st = t.get(sid).unwrap();
        assert_eq!(st.kind, LspStatusKind::Ready);
        assert_eq!(st.busy, None);
        assert_eq!(st.modeline_text(), "ready");
    }

    /// E7b.1: the whole startup family flips the kind, and a token the
    /// list does not know --- a number, or another server's name ---
    /// does not, whatever its title says.
    #[test]
    fn only_the_enumerated_tokens_flip_the_kind() {
        let now = Instant::now();
        for token in INDEXING_TOKENS {
            let sid = LspServerId::next();
            let mut t = ready_tracker(sid, now);
            t.observe(
                &progress(sid, token, &json!({ "kind": "begin", "title": "x" }), now),
                None,
            );
            assert!(
                matches!(t.get(sid).unwrap().kind, LspStatusKind::Indexing { .. }),
                "{token} must index"
            );
            t.observe(&progress(sid, token, &json!({ "kind": "end" }), now), None);
            assert_eq!(t.get(sid).unwrap().kind, LspStatusKind::Ready, "{token}");
        }
        for token in ["1", "rustAnalyzer/Roots Scanned/other", "pyright/analyzing"] {
            let sid = LspServerId::next();
            let mut t = ready_tracker(sid, now);
            t.observe(
                &progress(
                    sid,
                    token,
                    &json!({ "kind": "begin", "title": "Indexing" }),
                    now,
                ),
                None,
            );
            let st = t.get(sid).unwrap();
            assert_eq!(st.kind, LspStatusKind::Ready, "{token} must not index");
            assert_eq!(st.modeline_text(), "ready·indexing");
        }
    }

    /// E7b.1: a `Degraded` kind is disturbed by neither cycle. The
    /// indexing cycle is remembered, and is what the sticky window
    /// releases to.
    #[test]
    fn degraded_is_disturbed_by_neither_a_flycheck_nor_an_indexing_cycle() {
        let sid = LspServerId::next();
        let now = Instant::now();
        let mut t = ready_tracker(sid, now);
        t.observe(
            &ev(
                sid,
                LspEventKind::ProtocolError {
                    message: "bad frame".into(),
                },
                now,
            ),
            Some(&LspClientState::Initialized {
                capabilities: json!({}),
                server_info: None,
                initialized_at: now,
            }),
        );
        assert!(matches!(
            t.get(sid).unwrap().kind,
            LspStatusKind::Degraded { .. }
        ));
        for token in ["rust-analyzer/flycheck/0", "rustAnalyzer/cachePriming"] {
            t.observe(
                &progress(sid, token, &json!({ "kind": "begin", "title": "t" }), now),
                None,
            );
            assert!(
                matches!(t.get(sid).unwrap().kind, LspStatusKind::Degraded { .. }),
                "{token}'s begin must not disturb Degraded"
            );
        }
        t.observe(
            &progress(
                sid,
                "rust-analyzer/flycheck/0",
                &json!({ "kind": "end" }),
                now,
            ),
            None,
        );
        assert!(matches!(
            t.get(sid).unwrap().kind,
            LspStatusKind::Degraded { .. }
        ));
        assert_eq!(t.get(sid).unwrap().modeline_text(), "degraded");
        // The window closes with the indexing cycle still in flight:
        // that is what the kind returns to, and its end is what makes
        // the server ready.
        t.tick(now + DEGRADED_STICKY + Duration::from_secs(1), |_| {
            Some(LspClientState::Initialized {
                capabilities: json!({}),
                server_info: None,
                initialized_at: now,
            })
        });
        assert!(matches!(
            t.get(sid).unwrap().kind,
            LspStatusKind::Indexing { .. }
        ));
        t.observe(
            &progress(
                sid,
                "rustAnalyzer/cachePriming",
                &json!({ "kind": "end" }),
                now,
            ),
            None,
        );
        assert_eq!(t.get(sid).unwrap().kind, LspStatusKind::Ready);
    }

    /// E7b.1: two cycles in flight at once. The flycheck's end leaves
    /// the kind indexing; the indexing's end leaves the flycheck's
    /// suffix; a restart forgets both.
    #[test]
    fn overlapping_cycles_settle_by_token_and_a_restart_forgets_them() {
        let sid = LspServerId::next();
        let now = Instant::now();
        let mut t = ready_tracker(sid, now);
        let check = "rust-analyzer/flycheck/0";
        let prime = "rustAnalyzer/cachePriming";
        t.observe(
            &progress(
                sid,
                check,
                &json!({ "kind": "begin", "title": "cargo check" }),
                now,
            ),
            None,
        );
        t.observe(
            &progress(
                sid,
                prime,
                &json!({ "kind": "begin", "title": "Indexing" }),
                now,
            ),
            None,
        );
        assert_eq!(t.get(sid).unwrap().modeline_text(), "idx");
        assert_eq!(t.get(sid).unwrap().busy.as_deref(), Some("cargo check"));
        t.observe(&progress(sid, check, &json!({ "kind": "end" }), now), None);
        assert_eq!(t.get(sid).unwrap().modeline_text(), "idx");
        t.observe(
            &progress(
                sid,
                check,
                &json!({ "kind": "begin", "title": "cargo check" }),
                now,
            ),
            None,
        );
        t.observe(&progress(sid, prime, &json!({ "kind": "end" }), now), None);
        assert_eq!(t.get(sid).unwrap().modeline_text(), "ready·check");
        t.observe(&ev(sid, LspEventKind::Started { pid: 7 }, now), None);
        let st = t.get(sid).unwrap();
        assert_eq!(st.kind, LspStatusKind::Initializing);
        assert!(st.progress.is_empty());
        assert_eq!(st.busy, None);
    }

    #[test]
    fn busy_suffix_is_short() {
        assert_eq!(busy_suffix("cargo check"), "check");
        assert_eq!(busy_suffix("cargo clippy (#2)"), "clippy");
        assert_eq!(busy_suffix("Roots Scanned"), "roots");
        assert_eq!(busy_suffix("Building compile-time-deps"), "building");
        assert_eq!(busy_suffix(""), "busy");
    }

    #[test]
    fn crashed_event_records_last_error_and_kind() {
        let mut t = LspStatusTracker::new();
        let sid = LspServerId::next();
        let now = Instant::now();
        t.observe(
            &ev(
                sid,
                LspEventKind::Crashed {
                    reason: "exit 7".into(),
                },
                now,
            ),
            None,
        );
        let st = t.get(sid).unwrap();
        assert!(matches!(st.kind, LspStatusKind::Crashed { .. }));
        let err = st.last_error.as_ref().unwrap();
        assert_eq!(err.source, "crash");
        assert_eq!(err.message, "exit 7");
    }

    #[test]
    fn stderr_lines_split_on_newlines() {
        let mut t = LspStatusTracker::new();
        let sid = LspServerId::next();
        let now = Instant::now();
        t.observe(
            &ev(
                sid,
                LspEventKind::Stderr(b"first\nsecond\n\nthird\n".to_vec()),
                now,
            ),
            None,
        );
        let lines: Vec<_> = t
            .get(sid)
            .unwrap()
            .recent_messages
            .iter()
            .filter(|m| m.channel == "stderr")
            .map(|m| m.summary.clone())
            .collect();
        assert_eq!(lines, vec!["first", "second", "third"]);
    }

    #[test]
    fn ring_buffer_drops_oldest() {
        let mut t = LspStatusTracker::new();
        let sid = LspServerId::next();
        let now = Instant::now();
        // Force a small ring.
        let st = t.ensure(sid, now);
        st.message_capacity = 3;
        for i in 0..5 {
            t.observe(&ev(sid, LspEventKind::Started { pid: i }, now), None);
        }
        let st = t.get(sid).unwrap();
        assert_eq!(st.recent_messages.len(), 3);
        // Last message records the most recent pid (4).
        assert!(st.recent_messages.last().unwrap().summary.contains("pid 4"));
    }

    #[test]
    fn caps_summary_keys() {
        let v = json!({
            "textDocumentSync": 1,
            "hoverProvider": true,
            "completionProvider": { "triggerCharacters": ["."] },
            "definitionProvider": true,
            "diagnosticProvider": {}
        });
        let keys = caps_summary(&v);
        assert!(keys.contains(&"sync"));
        assert!(keys.contains(&"hover"));
        assert!(keys.contains(&"completion"));
        assert!(keys.contains(&"definition"));
        assert!(keys.contains(&"diagnostics"));
        assert!(!keys.contains(&"rename"));
    }

    #[test]
    fn format_status_buffer_includes_state_and_capabilities() {
        let mut t = LspStatusTracker::new();
        let sid = LspServerId::next();
        let now = Instant::now();
        t.observe(
            &ev(
                sid,
                LspEventKind::Initialized {
                    capabilities: json!({"hoverProvider": true, "serverInfo": {"name": "x"}}),
                },
                now,
            ),
            None,
        );
        let s = format_status_buffer(
            &t,
            |id| Some(format!("srv-{}", id.raw())),
            |_| Some(json!({"hoverProvider": true})),
            now,
        );
        assert!(s.contains("LSP servers"));
        assert!(s.contains("state=ready"));
        assert!(s.contains("hover"));
    }

    #[test]
    fn format_status_buffer_empty_branch() {
        let t = LspStatusTracker::new();
        let s = format_status_buffer(&t, |_| None, |_| None, Instant::now());
        assert!(s.contains("No active LSP servers"));
    }

    #[test]
    fn tick_releases_stale_degraded_when_layer_says_initialized() {
        let mut t = LspStatusTracker::new();
        let sid = LspServerId::next();
        let now = Instant::now();
        t.observe(
            &ev(
                sid,
                LspEventKind::Initialized {
                    capabilities: json!({}),
                },
                now,
            ),
            None,
        );
        t.observe(
            &ev(
                sid,
                LspEventKind::ProtocolError {
                    message: "bad".into(),
                },
                now,
            ),
            Some(&LspClientState::Initialized {
                capabilities: json!({}),
                server_info: None,
                initialized_at: now,
            }),
        );
        assert!(matches!(
            t.get(sid).unwrap().kind,
            LspStatusKind::Degraded { .. }
        ));
        let later = now + DEGRADED_STICKY + Duration::from_millis(1);
        t.tick(later, |_| {
            Some(LspClientState::Initialized {
                capabilities: json!({}),
                server_info: None,
                initialized_at: now,
            })
        });
        assert_eq!(t.get(sid).unwrap().kind, LspStatusKind::Ready);
    }

    /// E6d.2 --- a hundred keystrokes' worth of in-flight requests
    /// answered `ContentModified` (and a few `RequestCancelled`) leave
    /// a ready server ready, with no last error and nothing in the
    /// recent messages; a real error still degrades it. The row below
    /// extends it with the owner's ruling at C7b fix round 1.
    #[test]
    fn retry_codes_are_not_degradation_and_a_real_error_still_is() {
        let mut t = LspStatusTracker::new();
        let sid = LspServerId::next();
        let now = Instant::now();
        t.observe(
            &ev(
                sid,
                LspEventKind::Initialized {
                    capabilities: json!({}),
                },
                now,
            ),
            None,
        );
        let before = t.get(sid).unwrap().recent_messages.len();
        for i in 0..100u64 {
            let code = if i % 10 == 0 {
                REQUEST_CANCELLED
            } else {
                CONTENT_MODIFIED
            };
            t.observe(
                &ev(
                    sid,
                    LspEventKind::Response {
                        id: i,
                        result: Value::Null,
                        error: Some(LspError {
                            code,
                            message: "content modified".into(),
                            data: None,
                        }),
                        method: "textDocument/semanticTokens/range".into(),
                    },
                    now + Duration::from_millis(i),
                ),
                None,
            );
            let st = t.get(sid).unwrap();
            assert_eq!(st.kind, LspStatusKind::Ready, "keystroke {i}");
            assert!(st.last_error.is_none(), "keystroke {i} left a last error");
            assert_eq!(st.retry_responses, i + 1, "counted, not logged");
        }
        assert_eq!(
            t.get(sid).unwrap().recent_messages.len(),
            before,
            "retry codes are not messages either"
        );
        t.observe(
            &ev(
                sid,
                LspEventKind::Response {
                    id: 1000,
                    result: Value::Null,
                    error: Some(LspError {
                        code: -32603,
                        message: "internal error".into(),
                        data: None,
                    }),
                    method: "textDocument/hover".into(),
                },
                now + Duration::from_secs(1),
            ),
            None,
        );
        let st = t.get(sid).unwrap();
        assert!(matches!(st.kind, LspStatusKind::Degraded { .. }));
        assert_eq!(st.last_error.as_ref().unwrap().code, Some(-32603));
    }

    /// E6d.2's witness, extended at C7b fix round 1 with the owner's
    /// ruling that degraded means the server failed: the three
    /// client-caused codes (`-32600` `InvalidRequest`, `-32601`
    /// `MethodNotFound`, `-32602` `InvalidParams`) each log one
    /// `response error` line carrying the method and the code and
    /// leave the kind `Ready` with no last error, while a code in the
    /// server-error range (`-32002` `ServerNotInitialized`) degrades
    /// it as `-32603` does above. Bitten by hand:
    /// `is_server_failure_code` widened to `true` for every code fails
    /// the first client code's kind assertion.
    #[test]
    fn client_codes_leave_the_kind_alone_and_a_server_range_code_degrades_it() {
        let mut t = LspStatusTracker::new();
        let sid = LspServerId::next();
        let now = Instant::now();
        t.observe(
            &ev(
                sid,
                LspEventKind::Initialized {
                    capabilities: json!({}),
                },
                now,
            ),
            None,
        );
        // The client-caused codes: pmacs sent something wrong, the
        // server answered, and the server's health is unchanged. Each
        // one is a `*lsp*` line naming the method and carrying the
        // code, and nothing else moves.
        for (i, (code, name, method)) in [
            (-32600, "InvalidRequest", "textDocument/hover"),
            (-32601, "MethodNotFound", "textDocument/prepareRename"),
            (-32602, "InvalidParams", "textDocument/rename"),
        ]
        .into_iter()
        .enumerate()
        {
            let logged = t.get(sid).unwrap().recent_messages.len();
            t.observe(
                &ev(
                    sid,
                    LspEventKind::Response {
                        id: 500 + i as u64,
                        result: Value::Null,
                        error: Some(LspError {
                            code,
                            message: format!("{name} from the server"),
                            data: None,
                        }),
                        method: method.into(),
                    },
                    now + Duration::from_millis(500 + i as u64),
                ),
                None,
            );
            let st = t.get(sid).unwrap();
            assert_eq!(
                st.kind,
                LspStatusKind::Ready,
                "{code} {name} is the client's, not the server's"
            );
            assert!(
                st.last_error.is_none(),
                "{code} {name} arms no sticky window"
            );
            assert_eq!(st.retry_responses, 0, "{code} {name} is not a retry code");
            let last = st.recent_messages.last().unwrap();
            assert_eq!(
                st.recent_messages.len(),
                logged + 1,
                "{code} {name} logs one line"
            );
            assert_eq!(last.channel, "error");
            assert_eq!(last.summary, format!("response error: {method}"));
            assert_eq!(
                last.detail.as_deref(),
                Some(format!("{name} from the server (code {code})").as_str()),
                "the line carries the code"
            );
        }
        // A code in the server-error range is the server's failure.
        t.observe(
            &ev(
                sid,
                LspEventKind::Response {
                    id: 900,
                    result: Value::Null,
                    error: Some(LspError {
                        code: -32002,
                        message: "server not initialized".into(),
                        data: None,
                    }),
                    method: "textDocument/completion".into(),
                },
                now + Duration::from_millis(900),
            ),
            None,
        );
        let st = t.get(sid).unwrap();
        assert!(
            matches!(st.kind, LspStatusKind::Degraded { .. }),
            "-32002 ServerNotInitialized degrades: {:?}",
            st.kind
        );
        assert_eq!(st.last_error.as_ref().unwrap().code, Some(-32002));
    }

    /// The partition itself, code by code: the server-failure set is
    /// exactly `-32603` and `-32099..=-32000`; the client codes, the
    /// retry codes, `ParseError`, `ServerCancelled`, `RequestFailed`
    /// and an unknown code are outside it.
    #[test]
    fn the_server_failure_set_is_internal_error_and_the_server_range() {
        for code in [-32603, -32099, -32050, -32002, -32001, -32000] {
            assert!(is_server_failure_code(code), "{code} is the server's");
            assert!(!is_retry_code(code));
        }
        for code in [
            -32600, -32601, -32602, -32700, -32802, -32803, -32100, -31999, 0, 1,
        ] {
            assert!(!is_server_failure_code(code), "{code} is not the server's");
        }
        for code in [REQUEST_CANCELLED, CONTENT_MODIFIED] {
            assert!(is_retry_code(code));
            assert!(!is_server_failure_code(code));
        }
    }
}
