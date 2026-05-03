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
//!         │                  begin/Indexing ─→│
//!         │                                   │ end ←──┐
//!         │                                   ↑        │
//!         │                                   └────────┘
//!         │
//!         ├── ProtocolError / response.error ─→ Degraded { reason }
//!         │       ↑                                │
//!         │       └─ further errors stay degraded; │ next Initialized
//!         │                                        │ flips back to Ready
//!         │
//!         ├── Crashed event → Crashed { reason }
//!         └── ShuttingDown / Stopped → Stopped
//! ```
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
        }
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
                if let Some(err) = error {
                    st.last_error = Some(LspStatusError::from_lsp_error(ev.at, err));
                    st.set_kind(
                        LspStatusKind::Degraded {
                            reason: format!("{method}: {} (code {})", err.message, err.code),
                        },
                        ev.at,
                    );
                    st.push_message(LspStatusMessage {
                        at: ev.at,
                        channel: "error",
                        summary: format!("response error: {method}"),
                        detail: Some(err.message.clone()),
                    });
                }
            }
            LspEventKind::ShuttingDown => {
                st.set_kind(LspStatusKind::Stopped, ev.at);
                st.push_message(LspStatusMessage {
                    at: ev.at,
                    channel: "info",
                    summary: "shutting down".into(),
                    detail: None,
                });
            }
            LspEventKind::Stopped => {
                st.set_kind(LspStatusKind::Stopped, ev.at);
                st.push_message(LspStatusMessage {
                    at: ev.at,
                    channel: "info",
                    summary: "stopped".into(),
                    detail: None,
                });
            }
            LspEventKind::Crashed { reason } => {
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
                st.kind = LspStatusKind::Ready;
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
        kind,
        title,
        percentage,
        message,
    })
}

fn apply_progress(st: &mut LspStatus, at: Instant, update: &ProgressUpdate) {
    match update.kind {
        ProgressKind::Begin | ProgressKind::Report => {
            // Hold onto whatever title we already had if this update
            // doesn't carry one (LSP `report` updates are allowed to
            // drop the title).
            let title = update
                .title
                .clone()
                .or_else(|| match &st.kind {
                    LspStatusKind::Indexing { title, .. } => Some(title.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| "indexing".into());
            st.set_kind(
                LspStatusKind::Indexing {
                    title,
                    percentage: update.percentage,
                },
                at,
            );
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
            // End → Ready (only if we were actually indexing, so an
            // out-of-band `end` doesn't kick us out of `Degraded`).
            if matches!(st.kind, LspStatusKind::Indexing { .. }) {
                st.set_kind(LspStatusKind::Ready, at);
            }
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

    #[test]
    fn progress_begin_to_end_cycles_indexing() {
        let mut t = LspStatusTracker::new();
        let sid = LspServerId::next();
        let now = Instant::now();
        // Get to Ready first.
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
        // begin
        t.observe(
            &ev(
                sid,
                LspEventKind::Notification {
                    method: "$/progress".into(),
                    params: json!({
                        "token": 1,
                        "value": { "kind": "begin", "title": "indexing", "percentage": 5 }
                    }),
                },
                now,
            ),
            None,
        );
        let kind = t.get(sid).unwrap().kind.clone();
        assert!(matches!(kind, LspStatusKind::Indexing { ref title, .. } if title == "indexing"));
        // report
        t.observe(
            &ev(
                sid,
                LspEventKind::Notification {
                    method: "$/progress".into(),
                    params: json!({
                        "token": 1,
                        "value": { "kind": "report", "percentage": 50 }
                    }),
                },
                now,
            ),
            None,
        );
        if let LspStatusKind::Indexing { percentage, title } = &t.get(sid).unwrap().kind {
            assert_eq!(*percentage, Some(50));
            // Title from begin must persist across report w/o title.
            assert_eq!(title, "indexing");
        } else {
            panic!("expected Indexing");
        }
        // end → Ready
        t.observe(
            &ev(
                sid,
                LspEventKind::Notification {
                    method: "$/progress".into(),
                    params: json!({ "token": 1, "value": { "kind": "end" } }),
                },
                now,
            ),
            None,
        );
        assert_eq!(t.get(sid).unwrap().kind, LspStatusKind::Ready);
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
}
