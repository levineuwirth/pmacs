// attach.rs --- Frontend attach client for the local-attach transport.

//! Frontend `--attach` entry point (T M5.5g).
//!
//! Connects to a daemon's Unix socket, performs the
//! [`Hello`] / [`AttachRequest`] handshake, sets up the local terminal
//! with the same [`Frontend`] used by the in-process TUI, and then
//! pumps events bidirectionally:
//!
//! - Local input ([`crossterm::event::Event`]) → translated to
//!   [`FrontendEvent`] and written to the socket.
//! - Instance messages ([`InstanceMessage`]) read from the socket →
//!   applied to the [`Frontend`] (cell deltas, cursor moves).
//!
//! # Detach
//!
//! v0.1 uses **F12** as the detach key: a single keystroke that the
//! client intercepts (does not forward) and translates into
//! [`FrontendEvent::Detach`]. The daemon closes the connection
//! cleanly without sending a Goodbye (the frontend asked first).
//!
//! Closing the terminal (SIGHUP) also works — the daemon sees the
//! socket close and cleans up the per-attach state. The choice
//! between F12 and "just close the terminal" is up to the user;
//! both result in the same daemon-side cleanup.
//!
//! The F12 choice is tentative for v0.1; it conflicts with any user
//! keybind on F12, which we accept because F12 is rarely bound. A
//! later release will make it configurable.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread;
use std::time::Duration;

use std::time::Instant;

use crate::attach_reconnect::{
    BackoffSchedule, HANDSHAKE_RETRY_CAP, ReconnectVerdict, classify_for_reconnect,
};
use crate::cell::CellSize;
use crate::frontend::{Event, Frontend, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crate::protocol::crossterm_translate::{key_from_crossterm, mouse_from_crossterm};
use crate::protocol::{
    AttachRequest, AttachTarget, FrontendCapabilities, FrontendEvent, FrontendId, GoodbyeReason,
    Hello, InstanceMessage, PROTOCOL_VERSION,
};
use crate::transport::{TransportError, read_message, write_message};

/// Environment variable that overrides the SSH binary used by
/// [`run_attach_ssh`]. Test-flavored — production users do not set
/// this; users who want to substitute a non-`ssh` binary in v0.2+
/// will get a dedicated CLI flag.
pub const PMACS_TEST_SSH_BIN: &str = "PMACS_TEST_SSH_BIN";

/// Enable stderr breadcrumbs for SSH attach and the far-side
/// `--daemon-attach` bridge. The local side propagates this to the
/// remote command via `env PMACS_ATTACH_DEBUG=1 ...`, so users only
/// need to set it on the initiating shell.
const PMACS_ATTACH_DEBUG: &str = "PMACS_ATTACH_DEBUG";

/// Optional path for the full attach-debug log. When debug is
/// enabled, breadcrumbs are *always* appended here (including
/// live-session protocol reads, which must not hit the terminal the
/// TUI renders on). Unset → a default under the temp dir. Stderr
/// mirroring is additional and only happens before the interactive
/// frontend takes the terminal.
const PMACS_ATTACH_DEBUG_FILE: &str = "PMACS_ATTACH_DEBUG_FILE";

/// Explicit per-invocation override of the SSH protocol channel:
/// `stdout` / `1` or `stderr` / `2`. Unset → [`SSH_PROTOCOL_DEFAULT`].
/// This is the supported way to opt back to stdout (the F8b
/// "fallback") without a rebuild.
const PMACS_ATTACH_SSH_PROTOCOL: &str = "PMACS_ATTACH_SSH_PROTOCOL";

/// Legacy/back-compat override (pre-F8b name). `=1`/non-empty →
/// stderr, `=0` → stdout. Honored only when [`PMACS_ATTACH_SSH_PROTOCOL`]
/// is unset/unrecognized. New code/users should prefer the clearer
/// `PMACS_ATTACH_SSH_PROTOCOL`.
const PMACS_ATTACH_SSH_PROTOCOL_STDERR: &str = "PMACS_ATTACH_SSH_PROTOCOL_STDERR";

/// Which SSH channel carries the wire protocol.
///
/// **F8b (see `M10.11-AUDIT.md`).** At least one tested host
/// (`OpenSSH_10.3p1`) does not forward a live non-PTY remote process's
/// **stdout (fd1)** while it stays alive, but forwards **stderr
/// (fd2)** in real time. The `--daemon-attach` bridge is exactly
/// such a long-lived non-PTY process, so the protocol must ride
/// stderr there; stdout hangs forever. Evidence is n=1, so this is
/// deliberately a **single switch**: change [`SSH_PROTOCOL_DEFAULT`]
/// (one line) to flip the default if breadth evidence ever shows
/// stdout should win; nothing downstream needs to change. A
/// per-invocation override (`PMACS_ATTACH_SSH_PROTOCOL=stdout|stderr`)
/// selects without any rebuild.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SshProtocolChannel {
    Stdout,
    Stderr,
}

impl SshProtocolChannel {
    /// The remote fd the far-side bridge must write the protocol to.
    fn remote_fd(self) -> u8 {
        match self {
            Self::Stdout => 1,
            Self::Stderr => 2,
        }
    }
}

/// THE switch. Flip this one line to change the SSH-attach protocol
/// channel default; everything else derives from it.
const SSH_PROTOCOL_DEFAULT: SshProtocolChannel = SshProtocolChannel::Stderr;

/// Remote-side env var consumed by `daemon_attach.rs`.
const PMACS_ATTACH_PROTOCOL_FD: &str = "PMACS_ATTACH_PROTOCOL_FD";

/// Maximum bytes of remote stderr retained for diagnostic surfacing
/// in [`AttachError::SshChildExited`]. Sized to catch typical SSH
/// failure messages (one or two short lines plus optional banner)
/// without unbounded growth on a chatty remote.
const SSH_STDERR_TAIL_CAP: usize = 4096;

/// Watchdog timeout for the SSH-transport kick. After this delay,
/// the watchdog SIGTERMs the child to ensure the reader thread can
/// exit even if the EOF cascade is wedged. Typical clean-exit cases
/// complete well under this; the SIGTERM is harmless against an
/// already-exited child.
const SSH_KICK_WATCHDOG: Duration = Duration::from_secs(1);

/// Errors that abort the attach client.
#[derive(Debug)]
pub enum AttachError {
    /// Could not connect to the socket, or write to it.
    Io(std::io::Error),
    /// Transport-layer error (encode / decode / framing).
    Transport(TransportError),
    /// Daemon reports a different protocol version than ours.
    VersionMismatch {
        /// Protocol version the daemon advertised in `Hello`.
        server: u32,
        /// Protocol version this client supports.
        client: u32,
    },
    /// Daemon rejected the attach (already attached, etc.).
    Rejected(GoodbyeReason),
    /// Terminal-side error setting up or driving the TUI.
    Terminal(std::io::Error),
    /// `Command::spawn` of `ssh` (or the [`PMACS_TEST_SSH_BIN`]
    /// override) failed. The most common cause is the binary not
    /// being on `PATH`, which the error message names explicitly.
    SshSpawnFailed {
        /// Path or name of the binary the spawn attempted.
        command: PathBuf,
        /// The underlying `Command::spawn` error.
        source: std::io::Error,
    },
    /// SSH child exited non-zero. The exit code is classified into
    /// a user-facing diagnostic — code 127 in particular gets the
    /// "command not found on remote" treatment, since that's the
    /// failure mode users hit most often when their dotfiles set
    /// `PATH` only for interactive shells.
    SshChildExited {
        /// Exit code if available. `None` means the child was
        /// terminated by a signal.
        code: Option<i32>,
        /// Up to [`SSH_STDERR_TAIL_CAP`] bytes from the tail of the
        /// child's stderr. The bytes were also inherited to our
        /// stderr in real time (the user has likely already seen
        /// them in their terminal scrollback).
        stderr_tail: String,
    },
}

impl std::fmt::Display for AttachError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "attach I/O error: {e}"),
            Self::Transport(e) => write!(f, "{e}"),
            Self::VersionMismatch { server, client } => {
                // T M10.7 criterion 5: the message must tell the user
                // which side is at the older version. Comparing
                // `server` against `client` produces an unambiguous
                // identification without the user needing to decode
                // version-number semantics.
                let which_older = match server.cmp(client) {
                    std::cmp::Ordering::Less => {
                        " The pmacs daemon is at the older version — upgrade the daemon \
                         (or restart it after upgrading the pmacs binary)."
                    }
                    std::cmp::Ordering::Greater => {
                        " Your pmacs binary is at the older version — upgrade the binary."
                    }
                    std::cmp::Ordering::Equal => "",
                };
                write!(
                    f,
                    "protocol version mismatch (instance v{server}, client v{client}).{which_older}"
                )
            }
            Self::Rejected(reason) => match reason {
                // T M10.7: capability negotiation mismatch — name the
                // capabilities the frontend asked for that the
                // instance can't provide. The strings on the wire are
                // exactly the `FrontendCapabilities` field names
                // (e.g., `multi_frontend`); user-facing translation
                // happens here.
                GoodbyeReason::CapabilityMismatch { missing } => {
                    let translated: Vec<&str> = missing
                        .iter()
                        .map(|name| match name.as_str() {
                            "multi_frontend" => "multi-frontend collaboration",
                            "crdt_replica" => "CRDT replica participation",
                            other => other,
                        })
                        .collect();
                    write!(
                        f,
                        "instance does not support the requested capabilities: {}",
                        translated.join(", ")
                    )
                }
                _ => write!(f, "instance rejected attach: {reason:?}"),
            },
            Self::Terminal(e) => write!(f, "terminal error: {e}"),
            Self::SshSpawnFailed { command, source } => write!(
                f,
                "could not spawn {} for SSH attach: {source}; \
                 verify the binary is on PATH and executable",
                command.display(),
            ),
            Self::SshChildExited { code, stderr_tail } => {
                let tail_hint = if stderr_tail.trim().is_empty() {
                    String::new()
                } else {
                    format!(
                        "; details may appear in your terminal scrollback above \
                         (last bytes: {})",
                        format_stderr_tail_for_display(stderr_tail),
                    )
                };
                match code {
                    Some(127) => write!(
                        f,
                        "SSH session exited 127 (command not found on remote). \
                         This usually means `pmacs` is not on the remote PATH for \
                         non-interactive SSH. Try: `ssh <host> 'which pmacs'` to \
                         verify{tail_hint}"
                    ),
                    Some(c) => write!(f, "SSH session exited {c}{tail_hint}"),
                    None => write!(
                        f,
                        "SSH session terminated by signal (no exit code){tail_hint}"
                    ),
                }
            }
        }
    }
}

impl std::error::Error for AttachError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) | Self::Terminal(e) => Some(e),
            Self::Transport(e) => Some(e),
            Self::SshSpawnFailed { source, .. } => Some(source),
            Self::VersionMismatch { .. } | Self::Rejected(_) | Self::SshChildExited { .. } => None,
        }
    }
}

/// Trim the captured stderr tail to a single short line for
/// inclusion in the [`AttachError::SshChildExited`] message. The
/// full bytes are available via the `stderr_tail` field; this is
/// just a one-liner preview.
fn format_stderr_tail_for_display(tail: &str) -> String {
    let trimmed = tail.trim_end();
    let last_line = trimmed.lines().next_back().unwrap_or("");
    let max_len = 200;
    if last_line.len() <= max_len {
        last_line.to_string()
    } else {
        format!("...{}", &last_line[last_line.len() - max_len..])
    }
}

impl From<std::io::Error> for AttachError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<TransportError> for AttachError {
    fn from(e: TransportError) -> Self {
        Self::Transport(e)
    }
}

/// Transport-agnostic IO trio for the attach pump.
///
/// `run_attach_pair` consumes one of these and drives the bidirectional
/// protocol: instance messages flow in through `reader`, frontend
/// events flow out through `writer`, and `kick` is the tear-down
/// escape hatch.
///
/// # Construction
///
/// Each transport builds its own `AttachIo` from the primitives it has
/// available. The local-socket transport (M5.5g) clones a `UnixStream`
/// three ways and uses a kick-aware reader plus socket shutdown for
/// the kick. The SSH
/// transport (M5.7e) takes a child process's `stdout` and `stdin`
/// halves and uses `SIGTERM` to the child for the kick.
pub(crate) struct AttachIo {
    /// Reader half. Moved into the reader thread; that thread owns
    /// blocking reads of `InstanceMessage` frames until the channel
    /// disconnects or the underlying transport closes.
    pub reader: Box<dyn Read + Send>,
    /// Writer half. Stays on the main thread, used for outbound
    /// `FrontendEvent` writes.
    pub writer: Box<dyn Write>,
    /// Force the reader thread to exit promptly, by any means
    /// necessary.
    ///
    /// Called by the main thread after the message loop has decided
    /// to terminate, immediately before joining the reader thread.
    /// The reader thread is presumed to be blocked on a read; this
    /// wakes it.
    ///
    /// Implementations may be destructive. Callers do not distinguish
    /// — by the time the kick runs, the pump has already decided to
    /// exit.
    pub kick: Box<dyn FnOnce() + Send>,
}

impl std::fmt::Debug for AttachIo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AttachIo")
            .field("reader", &"<dyn Read>")
            .field("writer", &"<dyn Write>")
            .field("kick", &"<FnOnce>")
            .finish()
    }
}

/// Frontend operations the attach pump consumes.
///
/// Pulled out as a trait so unit tests can substitute a mock without
/// taking over a real terminal. The production impl is on
/// [`Frontend`]; tests in this module supply their own.
pub(crate) trait AttachPumpFrontend {
    fn present_messages(&mut self, msgs: &[InstanceMessage]) -> std::io::Result<()>;
    fn poll_event(&mut self, timeout: Duration) -> std::io::Result<Option<Event>>;
    fn size(&self) -> CellSize;

    /// T M10.10 Day 3 step 5 Path β — paint an optimistic insert
    /// at the terminal's current cursor position. Used when the
    /// optimistic-apply orchestrator landed a `CrdtOp` AND the
    /// mirror reports `cursor_at_end_of_line == true` for the active
    /// buffer.
    ///
    /// Default impl no-ops; the production `Frontend` overrides with
    /// the actual terminal-write path. Tests using stub frontends
    /// inherit the no-op (visual paint isn't being asserted at the
    /// unit-test level).
    ///
    /// Feature-gated: the orchestrator's call site is `#[cfg(feature =
    /// "crdt")]`; the trait method exists only in CRDT builds to keep
    /// the non-CRDT trait surface minimal.
    #[cfg(feature = "crdt")]
    fn paint_optimistic_insert(&mut self, _c: char) -> std::io::Result<()> {
        Ok(())
    }

    /// T M10.10 Day 3 step 5 Path β — paint an optimistic
    /// delete-back: erase the cell to the left of the cursor and
    /// retreat the cursor one column. Cells match what the daemon's
    /// `CellDelta` will eventually carry (last char of line becomes a
    /// space at the cursor position before the cursor returns).
    #[cfg(feature = "crdt")]
    fn paint_optimistic_delete_back(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl AttachPumpFrontend for Frontend {
    fn present_messages(&mut self, msgs: &[InstanceMessage]) -> std::io::Result<()> {
        Frontend::present_messages(self, msgs)
    }
    fn poll_event(&mut self, timeout: Duration) -> std::io::Result<Option<Event>> {
        Frontend::poll_event(self, timeout)
    }
    fn size(&self) -> CellSize {
        Frontend::size(self)
    }
    #[cfg(feature = "crdt")]
    fn paint_optimistic_insert(&mut self, c: char) -> std::io::Result<()> {
        Frontend::paint_optimistic_insert(self, c)
    }
    #[cfg(feature = "crdt")]
    fn paint_optimistic_delete_back(&mut self) -> std::io::Result<()> {
        Frontend::paint_optimistic_delete_back(self)
    }
}

/// Connect to the daemon at `socket_path` and run the attach client.
///
/// Returns when the daemon disconnects (clean detach, instance
/// shutdown, version mismatch, etc.) or when the user presses F12.
// Owned `PathBuf` lets callers hand the result of
// `resolve_socket_path` straight in; clippy's pedantic
// pass-by-value is wrong for this entry point.
#[allow(clippy::needless_pass_by_value)]
pub fn run_attach(socket_path: PathBuf) -> Result<(), AttachError> {
    let mut stream = UnixStream::connect(&socket_path)?;

    // Hello / AttachRequest handshake on the raw stream, before
    // raw mode engages. If the daemon refuses the attach (version
    // mismatch, malformed Hello, EOF), the error message reaches a
    // normal terminal — `Frontend::new` hasn't taken over yet.
    let hello: Hello = read_message(&mut stream)?;
    // T M10.5: relaxed from strict equality to range membership per
    // `§sec:m10-backward-compat`. A v1.0 frontend accepts a Hello
    // from a v0.1 daemon (protocol_version=1) and downgrades its own
    // request to match the server's version; symmetric to the daemon-
    // side relaxation. Versions outside `SUPPORTED_PROTOCOL_VERSIONS`
    // are still rejected.
    if !crate::protocol::is_supported_protocol_version(hello.protocol_version) {
        return Err(AttachError::VersionMismatch {
            server: hello.protocol_version,
            client: PROTOCOL_VERSION,
        });
    }
    print_attach_info(&hello);

    let (cols, rows) = crossterm::terminal::size().map_err(AttachError::Terminal)?;
    let initial_size = CellSize::new(u32::from(rows), u32::from(cols));

    // T M10.5: match the server's protocol version so a v1.0 frontend
    // connecting to a v0.1 daemon advertises protocol_version=1 in
    // its AttachRequest (the v0.1 daemon's strict-equality check will
    // accept). The frontend's runtime behavior on the wire is the
    // intersection of features both sides support.
    let req = AttachRequest {
        protocol_version: hello.protocol_version,
        frontend_capabilities: build_capabilities(),
        initial_size,
    };
    write_message(&mut stream, &req)?;

    // Take over the terminal. Frontend's Drop tears it down.
    let mut frontend = Frontend::new().map_err(AttachError::Terminal)?;

    let io = build_local_socket_io(stream)?;
    let result = run_attach_pair(io, &mut frontend, hello.assigned_frontend_id);

    // Frontend drops here; raw mode + alternate screen + mouse capture
    // all torn down before we return.
    drop(frontend);

    result
}

/// Build an [`AttachIo`] for a connected `UnixStream`.
///
/// The kick sets a shared flag and clones a third handle for
/// `shutdown(Both)`. Cloning may
/// fail (rare — the kernel is out of file descriptors), in which
/// case the caller propagates the error before raw mode engages.
fn build_local_socket_io(stream: UnixStream) -> Result<AttachIo, std::io::Error> {
    let reader = stream.try_clone()?;
    reader.set_nonblocking(true)?;
    let kick_handle = stream.try_clone()?;
    let kicked = Arc::new(AtomicBool::new(false));
    let reader_kicked = Arc::clone(&kicked);
    Ok(AttachIo {
        reader: Box::new(KickAwareUnixReader {
            stream: reader,
            kicked: reader_kicked,
        }),
        writer: Box::new(stream),
        kick: Box::new(move || {
            kicked.store(true, Ordering::SeqCst);
            let _ = kick_handle.shutdown(Shutdown::Both);
        }),
    })
}

/// Non-blocking poll-based reader with a kick flag.
///
/// # Wake semantics
///
/// This reader has two cooperating wake paths, only one of which is
/// load-bearing:
///
/// 1. **Atomic flag (correctness):** the reader runs a non-blocking
///    poll loop with a 10ms sleep between iterations. After the kick
///    sets `kicked`, the next loop iteration observes it and returns
///    `Ok(0)`. Worst-case wake latency is one poll cycle (~10ms).
///    This path is platform-independent and is the mechanism the
///    caller relies on for correctness.
///
/// 2. **`shutdown(Both)` on a sibling clone (best-effort speedup):**
///    if the reader happens to be inside `self.stream.read()` when
///    the kick fires, and the platform honors cross-clone shutdown
///    wakes, the read returns `Ok(0)` immediately and the loop
///    skips its sleep. This path is **not** load-bearing — Unix
///    socket cross-clone shutdown semantics are not portably
///    guaranteed, and any wake it provides is a bonus on top of
///    path 1.
///
/// In other words: the atomic flag wakes the reader; the shutdown
/// just shaves up to ~10ms off the wake when the platform plays
/// along. Tests asserting wake bounds should treat the budget as
/// "≤ one poll cycle plus scheduler jitter," not as a measure of
/// shutdown latency.
struct KickAwareUnixReader {
    stream: UnixStream,
    kicked: Arc<AtomicBool>,
}

impl Read for KickAwareUnixReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            match self.stream.read(buf) {
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    if self.kicked.load(Ordering::SeqCst) {
                        return Ok(0);
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                other => return other,
            }
        }
    }
}

// `pub` so the post-audit Finding 6 production-path test in
// `tests/m5_5_acceptance.rs` can verify the production caps directly
// rather than reconstructing them in the test (which is exactly the
// gap that allowed Finding 1 to survive M10.10's first audit —
// `attach_multi()`'s custom caps bypassed the production function).
//
// Not part of the stable public API; reserved for internal test use.
#[doc(hidden)]
pub fn build_capabilities() -> FrontendCapabilities {
    // The v0.1 TUI implements all of these; we report them honestly
    // so the daemon doesn't strip features that work fine.
    //
    // T M10.10 — `multi_frontend` and `crdt_replica` advertise the
    // M10.10 BufferMirror + optimistic-apply infrastructure. Gated
    // on the `crdt` Cargo feature because the relevant modules
    // (`buffer_mirror`, `optimistic`) are conditionally compiled.
    // A non-CRDT build's frontend can't bootstrap a mirror and
    // shouldn't claim it can. CRDT-feature builds advertise true;
    // the daemon's per-tick CursorByte + BufferSnapshot bootstrap +
    // CrdtOp routing are then negotiated correctly.
    FrontendCapabilities {
        synchronized_output: true,
        unicode_smp: true,
        true_color: true,
        mouse: true,
        bracketed_paste: true,
        terminal_kind: std::env::var("TERM").ok(),
        multi_frontend: cfg!(feature = "crdt"),
        crdt_replica: cfg!(feature = "crdt"),
        // T M11.1 — the v0.1/v1.0 TUI is a grid frontend, not a
        // semantic (layout-local) renderer. It never consumes the
        // SemanticFrame family. A future GPU/GUI frontend sets this
        // true; the TUI stays false.
        semantic_render: false,
    }
}

fn print_attach_info(hello: &Hello) {
    let id = &hello.instance_identity;
    let name = id.instance_name.as_deref().unwrap_or("pmacs");
    let hash = id
        .build_hash
        .as_deref()
        .map(|h| format!(" ({h})"))
        .unwrap_or_default();
    let uptime = format_uptime(id.uptime_secs);
    let _ = writeln!(
        std::io::stderr(),
        "pmacs: attached to {name} (pmacs {version}{hash}, running {uptime})",
        version = id.pmacs_version,
    );
    let _ = writeln!(
        std::io::stderr(),
        "       press F12 to detach (or close the terminal)"
    );
}

fn format_uptime(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// Bidirectional pump: forward local input to the writer, apply
/// instance messages to the terminal.
///
/// Threading: a background thread reads instance messages from the
/// reader half and forwards them on a channel. The main thread polls
/// the terminal for input, drains the channel, and renders.
///
/// # Preconditions
///
/// Callers must complete the [`Hello`] / [`AttachRequest`] handshake
/// on the raw transport *before* calling this function. The next
/// bytes on `io.reader` are protocol traffic — typed
/// [`InstanceMessage`] frames. The handshake happens at the
/// construction site so handshake errors reach a normal terminal,
/// while pump errors happen behind raw mode.
///
/// # Tear-down contract
///
/// On any exit path — clean Goodbye, reader EOF, F12 detach, IO
/// error, or terminal error — `io.kick` fires and `reader_handle`
/// is joined before this function returns. The closure-and-call
/// wind-down pattern guarantees this regardless of which `return`
/// the loop takes.
// M10.10 grew this function with optimistic-apply orchestration +
// BufferSnapshot/CursorByte/CrdtOp routing in the message-drain
// loop. The 146-line size is intentionally cohesive: the closure
// captures the AttachIo writer and BufferMirror together, and
// splitting would require parameterizing both across helper
// functions or restructuring the wind-down pattern (drop(writer)
// → kick → join) which is the function's primary correctness
// invariant. The lint flags growth without naming a structural
// problem; defer to v0.2+ refactor if growth continues.
#[allow(clippy::too_many_lines)]
pub(crate) fn run_attach_pair(
    io: AttachIo,
    frontend: &mut dyn AttachPumpFrontend,
    assigned_id: FrontendId,
) -> Result<(), AttachError> {
    let AttachIo {
        reader,
        mut writer,
        kick,
    } = io;

    let (tx, rx) = mpsc::channel::<InstanceMessage>();
    let reader_handle = thread::spawn(move || run_reader(reader, tx));

    // T M10.10: per-session CRDT replica state. Bootstrapped by
    // `InstanceMessage::BufferSnapshot` messages routed in the
    // drain loop below; consumed by the optimistic-apply predicate
    // wired in Day 3.
    #[cfg(feature = "crdt")]
    let mut buffer_mirror = crate::buffer_mirror::BufferMirror::new(assigned_id);

    // Closure-and-call: any `return` from this closure still falls
    // through to `kick()` and `reader_handle.join()` below. Without
    // this wrapping a writer-side IO error inside the loop would skip
    // the wind-down.
    let result: Result<(), AttachError> = (|| {
        loop {
            // Drain instance messages. Goodbye exits immediately;
            // BufferSnapshot routes to the mirror; other messages are
            // batched into a single present_messages call.
            let mut batch: Vec<InstanceMessage> = Vec::new();
            let mut goodbye: Option<GoodbyeReason> = None;
            let mut reader_eof = false;
            loop {
                match rx.try_recv() {
                    Ok(InstanceMessage::Goodbye(reason)) => {
                        goodbye = Some(reason);
                        break;
                    }
                    #[cfg(feature = "crdt")]
                    Ok(InstanceMessage::BufferSnapshot {
                        buffer_id,
                        crdt_snapshot,
                    }) => {
                        // T M10.10: bootstrap the mirror for
                        // `buffer_id`. AlreadyInitialized errors
                        // surface a daemon-side bug (double-send) but
                        // shouldn't abort the session — log and
                        // continue with prior state. Loro decode
                        // errors are similarly logged.
                        if let Err(e) = buffer_mirror.init_from_snapshot(buffer_id, &crdt_snapshot)
                        {
                            eprintln!("pmacs: BufferMirror init for {buffer_id:?} failed: {e}");
                        }
                    }
                    #[cfg(feature = "crdt")]
                    Ok(InstanceMessage::CursorByte {
                        buffer_id,
                        byte_pos,
                    }) => {
                        // T M10.10 Finding 2: authoritative cursor
                        // byte-position update from the daemon. The
                        // optimistic-apply path consults
                        // `buffer_mirror.cursor_byte_pos(buffer_id)`
                        // before generating a local CrdtOp; this
                        // keeps that lookup current. Convert wire
                        // u64 → usize for the loro API.
                        buffer_mirror.set_cursor_byte_pos(buffer_id, byte_pos as usize);
                    }
                    #[cfg(feature = "crdt")]
                    Ok(InstanceMessage::CrdtOp { buffer_id, op }) => {
                        // T M10.10 step 4 — remote CrdtOp routing.
                        //
                        // The filter site is the message loop, NOT
                        // inside BufferMirror. Echoes of locally-
                        // applied edits arrive via CrdtOp broadcasts
                        // (the daemon fans out every op including the
                        // originator's own); the mirror has already
                        // applied these via apply_local_insert /
                        // apply_local_delete at keystroke time;
                        // re-applying would double-insert. The
                        // BufferMirror layer stays identity-ignorant
                        // by design — `apply_incoming_crdt_op` does
                        // the FrontendId comparison before invoking
                        // the mirror.
                        //
                        // Source FrontendId is derived from
                        // `op.peer_id` via the identity mapping
                        // documented in `crdt::peer_id_from_frontend`
                        // (FrontendId(n).0 == n).
                        let source = FrontendId(op.peer_id);
                        match crate::optimistic::apply_incoming_crdt_op(
                            &mut buffer_mirror,
                            assigned_id,
                            source,
                            buffer_id,
                            &op.bytes,
                        ) {
                            Ok(_outcome) => {
                                // Applied or SkippedEcho — both are
                                // success. Paint reconciliation
                                // (step 5) handles the visible diff.
                            }
                            Err(e) => {
                                // NotReady is the common case for a
                                // buffer this frontend hasn't been
                                // snapshotted for (mid-session
                                // buffer creation; v0.2's broadcast
                                // will close this gap). Log and
                                // continue.
                                eprintln!("pmacs: CrdtOp routing for {buffer_id:?} failed: {e}");
                            }
                        }
                    }
                    Ok(msg) => batch.push(msg),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        reader_eof = true;
                        break;
                    }
                }
            }
            if !batch.is_empty()
                && let Err(e) = frontend.present_messages(&batch)
            {
                return Err(AttachError::Terminal(e));
            }
            if let Some(reason) = goodbye {
                // Stderr buffered behind raw mode; surfaces after
                // Frontend's Drop runs.
                eprintln!("pmacs: instance disconnected: {reason:?}");
                return Ok(());
            }
            if reader_eof {
                eprintln!("pmacs: instance closed connection");
                return Ok(());
            }

            let event = match frontend.poll_event(Duration::from_millis(50)) {
                Ok(opt) => opt,
                Err(e) => return Err(AttachError::Terminal(e)),
            };
            let Some(ev) = event else {
                continue;
            };

            if is_detach_key(&ev) {
                let _ = write_message(&mut writer, &FrontendEvent::Detach(assigned_id));
                return Ok(());
            }

            // T M10.10 Day 3 step 3b — text-input optimistic-apply
            // orchestration. For Press/Repeat key events, the
            // orchestrator either:
            //  - returns FrontendEvent::CrdtOp (after applying the
            //    edit to the local mirror) when the mirror is ready
            //    for the active buffer, or
            //  - returns FrontendEvent::Key (graceful Refinement 4
            //    fallback) when the optimistic path isn't viable.
            // The caller writes whatever event was produced. Other
            // event kinds (mouse, resize, paste, focus, Release-kind
            // keys) fall through to the existing forward_event path.
            #[cfg(feature = "crdt")]
            let optimistic_handled = if let Event::Key(k) = &ev {
                if matches!(k.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                    let timestamp_ns = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(0));
                    let pmacs_key = key_from_crossterm(k, assigned_id, timestamp_ns);

                    // T M10.10 Day 3 step 5 Path β — determine
                    // visual-paint eligibility BEFORE the orchestrator
                    // mutates the mirror's cursor. End-of-line typing
                    // is the only case where single-Print optimistic
                    // paint matches the daemon's eventual CellDelta
                    // exactly (no cells right of cursor to shift). The
                    // action enum is captured so we can dispatch to
                    // the right paint primitive after the orchestrator
                    // produces a CrdtOp.
                    let action = crate::optimistic::classify_key(pmacs_key.key, pmacs_key.mods);
                    // Insert paint requires cursor at end-of-line (the
                    // single-Print sequence matches the daemon's
                    // eventual CellDelta exactly).
                    // Delete-back paint requires the stricter
                    // `cursor_at_end_of_line_safe_for_delete_back`
                    // predicate per post-audit Finding 5: also requires
                    // prev char != '\n'. Backspace that joins lines
                    // (prev char = newline) can't be represented by the
                    // single-column-erase paint sequence; falls
                    // through to v0.1 round-trip.
                    let active_buf = buffer_mirror.active_buffer();
                    let insert_paint_eligible = active_buf
                        .and_then(|b| buffer_mirror.cursor_at_end_of_line(b))
                        == Some(true);
                    let delete_back_paint_eligible = active_buf
                        .and_then(|b| buffer_mirror.cursor_at_end_of_line_safe_for_delete_back(b))
                        == Some(true);

                    let frontend_event = crate::optimistic::frontend_event_for_keystroke(
                        &mut buffer_mirror,
                        assigned_id,
                        pmacs_key,
                    );
                    if let Err(e) = write_message(&mut writer, &frontend_event) {
                        eprintln!("pmacs: write keystroke failed: {e}");
                        return Err(AttachError::from(e));
                    }
                    // Post-audit-round-4 F22 — if we round-tripped via
                    // `FrontendEvent::Key`, the daemon's command
                    // pipeline may move the cursor in ways the mirror
                    // can't predict locally (motion, Enter/Tab,
                    // mid-line edits, delete-forward, etc.). Mark the
                    // active buffer's cursor stale so subsequent
                    // keystrokes round-trip too until the daemon's
                    // next `CursorByte` re-grounds the mirror cursor.
                    if matches!(frontend_event, FrontendEvent::Key(_))
                        && let Some(active_buf) = buffer_mirror.active_buffer()
                    {
                        buffer_mirror.mark_cursor_stale(active_buf);
                    }

                    // Visual optimistic paint (Path β). Fires only when
                    // the orchestrator landed a CrdtOp (mirror was
                    // ready, action was optimistic-eligible) AND the
                    // pre-edit cursor was at an action-specific safe
                    // position. Mid-line operations, line-joining
                    // backspace, and round-trip cases skip — the
                    // daemon's CellDelta drives paint for those.
                    //
                    // Daemon-side CellDelta suppression is NOT needed:
                    // under Path β, optimistic paint either matches
                    // the eventual CellDelta exactly (end-of-line)
                    // or doesn't exist (mid-line / line-join). Either
                    // way, no flicker.
                    if matches!(frontend_event, FrontendEvent::CrdtOp { .. }) {
                        let paint_result = match action {
                            crate::optimistic::OptimisticAction::Insert(c)
                                if insert_paint_eligible =>
                            {
                                frontend.paint_optimistic_insert(c)
                            }
                            crate::optimistic::OptimisticAction::DeleteBack
                                if delete_back_paint_eligible =>
                            {
                                frontend.paint_optimistic_delete_back()
                            }
                            _ => Ok(()),
                        };
                        if let Err(e) = paint_result {
                            eprintln!("pmacs: optimistic paint failed: {e}");
                        }
                    }
                    true
                } else {
                    false
                }
            } else {
                false
            };
            #[cfg(not(feature = "crdt"))]
            let optimistic_handled = false;

            if !optimistic_handled
                && let Err(e) = forward_event(&mut writer, &ev, assigned_id, frontend.size())
            {
                // Likely a broken pipe — instance went away.
                eprintln!("pmacs: {e}");
                return Err(e);
            }
            // Post-audit-round-6 F30 — `forward_event` (success
            // path) may write a Mouse / Paste / Resize /
            // FocusGained / FocusLost event (or no-op for a Key
            // Release). Mouse down/drag in particular can move
            // the daemon's active window cursor, change the
            // active buffer, or both. Anything except an
            // optimistic CrdtOp can desync the mirror's cursor
            // from the daemon's view; conservatively mark the
            // active buffer's cursor stale so subsequent
            // keystrokes round-trip until the daemon's next
            // `CursorByte` re-grounds the mirror.
            #[cfg(feature = "crdt")]
            if let Some(active_buf) = buffer_mirror.active_buffer() {
                buffer_mirror.mark_cursor_stale(active_buf);
            }
        }
    })();

    // Wind down. The order matters:
    //
    // 1. `drop(writer)` — close our side of the protocol stream. For
    //    transports where the writer is the only handle to that
    //    half (SSH `ChildStdin`), this triggers an EOF cascade that
    //    ends with the reader thread seeing EOF naturally. For the
    //    local-socket transport, dropping one of three `UnixStream`
    //    clones is a no-op (other clones keep the FD alive); the
    //    reader needs `kick` to wake.
    //
    // 2. `kick()` — wake the reader thread, by any means necessary
    //    (per the kick contract). For local-socket: `shutdown(Both)`.
    //    For SSH: a watchdog that SIGTERMs the child if the EOF
    //    cascade hasn't reached the reader within the watchdog's
    //    grace period.
    //
    // 3. `join` — wait for the reader thread to exit.
    //
    // The drop-before-kick ordering is what lets SSH's clean-detach
    // path produce exit code 0: writer close starts the cascade,
    // kick is best-effort backup, the cascade typically completes
    // before kick has any effect.
    drop(writer);
    kick();
    let _ = reader_handle.join();

    result
}

/// F12 is the v0.1 detach key. Any modifier combination triggers
/// detach so users with sticky modifier keys can still exit.
fn is_detach_key(ev: &Event) -> bool {
    matches!(
        ev,
        Event::Key(crate::frontend::KeyEvent {
            code: KeyCode::F(12),
            kind: KeyEventKind::Press | KeyEventKind::Repeat,
            ..
        })
    )
}

fn forward_event<W: Write>(
    writer: &mut W,
    ev: &Event,
    assigned_id: FrontendId,
    term_size: CellSize,
) -> Result<(), AttachError> {
    let _ = term_size; // reserved for future use (Resize coordinate-ref)
    match ev {
        Event::Key(k) => {
            if !matches!(k.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                return Ok(());
            }
            // Strip Caps/Scroll/NumLock and so on — they shouldn't
            // pollute the modifier mask. The protocol only knows
            // SHIFT/CTRL/ALT/META/HYPER. Crossterm's translation
            // already handles this; we just need a stable timestamp.
            let _ = KeyModifiers::empty(); // import touch: keeps lint happy if unused
            let timestamp_ns = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(0));
            let pmacs_key = key_from_crossterm(k, assigned_id, timestamp_ns);
            write_message(writer, &FrontendEvent::Key(pmacs_key))?;
        }
        Event::Mouse(m) => {
            let pmacs_mouse = mouse_from_crossterm(m, assigned_id);
            write_message(writer, &FrontendEvent::Mouse(pmacs_mouse))?;
        }
        Event::Resize(cols, rows) => {
            let size = CellSize::new(u32::from(*rows), u32::from(*cols));
            write_message(
                writer,
                &FrontendEvent::Resize {
                    frontend_id: assigned_id,
                    size,
                },
            )?;
        }
        Event::Paste(data) => {
            write_message(
                writer,
                &FrontendEvent::Paste {
                    frontend_id: assigned_id,
                    data: data.clone().into_bytes(),
                },
            )?;
        }
        Event::FocusGained => {
            write_message(writer, &FrontendEvent::FocusGained(assigned_id))?;
        }
        Event::FocusLost => {
            write_message(writer, &FrontendEvent::FocusLost(assigned_id))?;
        }
    }
    Ok(())
}

// Same rationale as `daemon::run_reader`: reader is owned so the fd
// closes when the thread returns; tx is owned so the channel
// disconnects.
//
// Generic over any `Read + Send` so the same loop drives a
// `UnixStream` clone (local-socket transport) or a `ChildStdout`
// (SSH transport, M5.7e) without per-transport branching.
#[allow(clippy::needless_pass_by_value)]
fn run_reader(mut reader: Box<dyn Read + Send>, tx: mpsc::Sender<InstanceMessage>) {
    loop {
        match read_message::<InstanceMessage>(&mut reader) {
            Ok(msg) => {
                if tx.send(msg).is_err() {
                    return;
                }
            }
            Err(_) => return,
        }
    }
}

// ---------------------------------------------------------------------------
// SSH transport (T M5.7e)
// ---------------------------------------------------------------------------

/// Resolve the SSH binary the transport will spawn.
///
/// Production uses plain `ssh` (resolved through `PATH` by
/// `Command::new`). Tests may set [`PMACS_TEST_SSH_BIN`] to
/// substitute a stand-in that simulates the SSH child without
/// needing a real network or sshd.
fn ssh_binary() -> PathBuf {
    std::env::var_os(PMACS_TEST_SSH_BIN).map_or_else(|| PathBuf::from("ssh"), PathBuf::from)
}

fn attach_debug_enabled() -> bool {
    std::env::var_os(PMACS_ATTACH_DEBUG).is_some_and(|v| !v.is_empty() && v != "0")
}

/// Pure channel resolver (env-free core, unit-tested without env
/// mutation). Precedence: explicit `PMACS_ATTACH_SSH_PROTOCOL` >
/// legacy `PMACS_ATTACH_SSH_PROTOCOL_STDERR` > `default`.
fn resolve_ssh_protocol(
    explicit: Option<&str>,
    legacy: Option<&str>,
    default: SshProtocolChannel,
) -> SshProtocolChannel {
    if let Some(e) = explicit {
        match e.trim().to_ascii_lowercase().as_str() {
            "stdout" | "1" => return SshProtocolChannel::Stdout,
            "stderr" | "2" => return SshProtocolChannel::Stderr,
            _ => {} // unrecognized → fall through
        }
    }
    if let Some(l) = legacy {
        let l = l.trim();
        if !l.is_empty() {
            return if l == "0" {
                SshProtocolChannel::Stdout
            } else {
                SshProtocolChannel::Stderr
            };
        }
    }
    default
}

/// Env-reading wrapper over [`resolve_ssh_protocol`]. The single
/// place env is consulted for the channel decision.
fn ssh_protocol_channel() -> SshProtocolChannel {
    resolve_ssh_protocol(
        std::env::var(PMACS_ATTACH_SSH_PROTOCOL).ok().as_deref(),
        std::env::var(PMACS_ATTACH_SSH_PROTOCOL_STDERR)
            .ok()
            .as_deref(),
        SSH_PROTOCOL_DEFAULT,
    )
}

/// True once the interactive frontend owns the terminal (raw mode +
/// alternate screen). After that point, attach breadcrumbs must not
/// touch stderr — it *is* the screen the renderer paints — so they go
/// to the log file only. Stays true across reconnects (the TUI is
/// still up while a reconnect handshake runs).
static TUI_TERMINAL_OWNED: AtomicBool = AtomicBool::new(false);

/// Lazily-opened append handle for the full debug log. `None` inside
/// the `Option` means open failed; debug then degrades to stderr-only
/// (best-effort — diagnostics never abort the attach).
static DEBUG_FILE: OnceLock<Mutex<Option<std::fs::File>>> = OnceLock::new();

/// Call right before handing the terminal to the interactive pump so
/// subsequent breadcrumbs stop mirroring to stderr.
fn mark_tui_terminal_owned() {
    TUI_TERMINAL_OWNED.store(true, Ordering::Relaxed);
}

fn debug_file_path() -> PathBuf {
    std::env::var_os(PMACS_ATTACH_DEBUG_FILE).map_or_else(
        || std::env::temp_dir().join("pmacs-attach-debug.log"),
        PathBuf::from,
    )
}

fn debug_file() -> &'static Mutex<Option<std::fs::File>> {
    DEBUG_FILE.get_or_init(|| {
        let path = debug_file_path();
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            Ok(f) => {
                // Emitted during the handshake phase (terminal not yet
                // owned), so the user sees where the full log lives.
                eprintln!("pmacs attach debug: logging to {}", path.display());
                Mutex::new(Some(f))
            }
            Err(e) => {
                eprintln!(
                    "pmacs attach debug: could not open {} ({e}); stderr only",
                    path.display()
                );
                Mutex::new(None)
            }
        }
    })
}

fn attach_debug(msg: impl AsRef<str>) {
    if !attach_debug_enabled() {
        return;
    }
    let line = format!("pmacs attach debug: {}", msg.as_ref());
    // Always append the full stream to the log file — including the
    // live-session protocol reads that would otherwise paint over the
    // TUI's alternate screen.
    if let Ok(mut guard) = debug_file().lock()
        && let Some(f) = guard.as_mut()
    {
        let _ = writeln!(f, "{line}");
    }
    // Mirror to stderr only while it is still a normal terminal —
    // i.e., before the interactive frontend takes it over.
    if !TUI_TERMINAL_OWNED.load(Ordering::Relaxed) {
        eprintln!("{line}");
    }
}

fn hex_preview(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for (idx, byte) in bytes.iter().take(16).enumerate() {
        if idx > 0 {
            out.push(' ');
        }
        let _ = write!(&mut out, "{byte:02x}");
    }
    if bytes.len() > 16 {
        out.push_str(" ...");
    }
    out
}

struct DebugReader<R> {
    label: &'static str,
    inner: R,
    total: u64,
    logged_chunks: u8,
    max_logged_chunks: u8,
}

impl<R> DebugReader<R> {
    fn new(label: &'static str, inner: R, max_logged_chunks: u8) -> Self {
        Self {
            label,
            inner,
            total: 0,
            logged_chunks: 0,
            max_logged_chunks,
        }
    }
}

impl<R: Read> Read for DebugReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n > 0 {
            self.total += n as u64;
        }
        if attach_debug_enabled() {
            if n == 0 {
                attach_debug(format!("{}: EOF after {} bytes", self.label, self.total));
            } else if self.logged_chunks < self.max_logged_chunks {
                self.logged_chunks = self.logged_chunks.saturating_add(1);
                attach_debug(format!(
                    "{}: read chunk {} bytes (total {}), first bytes [{}]",
                    self.label,
                    n,
                    self.total,
                    hex_preview(&buf[..n])
                ));
                if self.logged_chunks == self.max_logged_chunks {
                    attach_debug(format!(
                        "{}: suppressing further chunk logs after {} chunks",
                        self.label, self.max_logged_chunks
                    ));
                }
            }
        }
        Ok(n)
    }
}

/// Construct the [`Command`] for SSH attach without spawning.
///
/// Pure (modulo the env-var lookup for test substitution): used
/// directly by [`run_attach_ssh`] and by unit tests that want to
/// assert argument shape without spawning a real `ssh`.
///
/// Argument order: `-T [user-flag] host [env VAR=val] pmacs
/// --daemon-attach [--socket NAME]`. The user flag is `-l USER` if
/// `user` is set;
/// SSH's own `~/.ssh/config` is consulted by the binary, so we
/// don't try to second-guess host aliases here.
///
/// # Panics
///
/// Does not panic on its own; passing a non-`Ssh` `AttachTarget`
/// returns `None`.
pub(crate) fn build_ssh_command(target: &AttachTarget) -> Option<Command> {
    let AttachTarget::Ssh {
        host,
        user,
        instance_name,
    } = target
    else {
        return None;
    };
    let mut cmd = Command::new(ssh_binary());
    // This is a binary stdio transport. Force no remote pseudo-tty
    // even if the user's ssh_config requests one for the host.
    cmd.arg("-T");
    if let Some(u) = user {
        cmd.arg("-l").arg(u);
    }
    cmd.arg(host);
    // Single source of truth (F8b): the resolved channel drives the
    // remote bridge's protocol fd. Stdout is fd 1 (the bridge's
    // default when unset), so we only pass the fd env when it
    // differs — keeps the stdout path's argv minimal and unchanged.
    // Debug is independent and additive (the remote bridge already
    // self-suppresses its stderr breadcrumbs when the protocol rides
    // fd 2, so the two can coexist; the old mutually-exclusive
    // `else if` could not).
    let mut env_pairs: Vec<String> = Vec::new();
    let fd = ssh_protocol_channel().remote_fd();
    if fd != 1 {
        env_pairs.push(format!("{PMACS_ATTACH_PROTOCOL_FD}={fd}"));
    }
    if attach_debug_enabled() {
        env_pairs.push(format!("{PMACS_ATTACH_DEBUG}=1"));
    }
    if !env_pairs.is_empty() {
        cmd.arg("env");
        for kv in &env_pairs {
            cmd.arg(kv);
        }
    }
    cmd.arg("pmacs").arg("--daemon-attach");
    if let Some(name) = instance_name {
        cmd.arg("--socket").arg(name);
    }
    Some(cmd)
}

/// Spawn a thread that copies child stderr to our own stderr while
/// retaining the tail bytes for diagnostic surfacing.
///
/// "Inherit always" was the explicit user requirement (so SSH
/// errors land in the user's terminal scrollback for diagnosis),
/// but we also wanted a tail for [`AttachError::SshChildExited`].
/// The tee thread does both when `tee_to_stderr` is `true`.
///
/// Returns the join handle and a shared ring buffer; the buffer
/// contains the most recent [`SSH_STDERR_TAIL_CAP`] bytes when the
/// thread exits.
///
/// `tee_to_stderr` controls whether SSH stderr is forwarded to our
/// real stderr in real time. The first attach attempt sets this to
/// `true` so handshake-time errors (DNS lookup, host key, auth) reach
/// the user's scrollback before raw mode engages. Reconnect attempts
/// (T M5.8d) set it to `false`: raw mode is already on, so live
/// stderr would corrupt the cell grid; the tail is still captured for
/// the give-up message.
fn spawn_stderr_tee(
    mut stderr: std::process::ChildStderr,
    tee_to_stderr: bool,
) -> (thread::JoinHandle<()>, Arc<Mutex<VecDeque<u8>>>) {
    let tail: Arc<Mutex<VecDeque<u8>>> =
        Arc::new(Mutex::new(VecDeque::with_capacity(SSH_STDERR_TAIL_CAP)));
    let tail_for_thread = tail.clone();
    let handle = thread::spawn(move || {
        let mut buf = [0u8; 1024];
        let mut local_stderr = if tee_to_stderr {
            Some(std::io::stderr().lock())
        } else {
            None
        };
        loop {
            let n = match stderr.read(&mut buf) {
                Ok(n) if n > 0 => n,
                _ => break,
            };
            if let Some(s) = local_stderr.as_mut() {
                let _ = s.write_all(&buf[..n]);
                let _ = s.flush();
            }
            let mut tail = tail_for_thread.lock().expect("tail mutex");
            for &b in &buf[..n] {
                if tail.len() >= SSH_STDERR_TAIL_CAP {
                    tail.pop_front();
                }
                tail.push_back(b);
            }
        }
    });
    (handle, tail)
}

/// Build the SSH-transport kick.
///
/// The kick spawns a watchdog thread: after [`SSH_KICK_WATCHDOG`]
/// elapses, SIGTERM the child if it's still running. In the
/// expected case (clean detach via Detach → daemon Goodbye → EOF
/// cascade), the child has already exited 0 well before the
/// watchdog fires; the SIGTERM goes to a non-existent process and
/// is harmlessly ignored. In the wedged case (network split,
/// daemon hung), the SIGTERM unsticks the reader thread.
///
/// Per the kick contract, this is best-effort: if SIGTERM itself
/// fails (ESRCH because the child already exited, EPERM in some
/// container scenarios), the error is dropped. The reader thread
/// will exit either way (cascade or signal).
fn ssh_kick(child_pid: u32) -> Box<dyn FnOnce() + Send> {
    Box::new(move || {
        thread::spawn(move || {
            thread::sleep(SSH_KICK_WATCHDOG);
            // Cast to nix's `Pid` (an `i32` newtype). On Linux the
            // PID space fits in `i32`; the cast is lossless for any
            // real process ID.
            #[allow(clippy::cast_possible_wrap)]
            let pid = nix::unistd::Pid::from_raw(child_pid as i32);
            let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM);
        });
    })
}

/// Connect to a remote daemon over SSH and run the attach client.
///
/// Spawns `ssh [-l USER] HOST pmacs --daemon-attach [--socket NAME]`,
/// performs the [`Hello`]/[`AttachRequest`] handshake on the child's
/// stdio (so handshake errors reach a normal terminal before raw
/// mode engages), then drives the standard pump. On exit, the
/// child is reaped and its exit code is classified into a
/// user-visible diagnostic — exit 127 in particular maps to a
/// "command not found on remote" message that names the most
/// likely cause (PATH missing in non-interactive SSH).
///
/// Stderr is **inherited and tee'd**: bytes flow to our stderr in
/// real time (so users see SSH errors in their scrollback) and
/// also into a ring buffer surfaced through
/// [`AttachError::SshChildExited`] for diagnostic context.
// Owned `AttachTarget` matches `run_attach`'s `PathBuf` and lets
// callers hand the dispatcher's `RunAttachSsh(target)` straight in;
// pedantic clippy is wrong for this seam.
#[allow(clippy::needless_pass_by_value)]
pub fn run_attach_ssh(target: AttachTarget) -> Result<(), AttachError> {
    // Pre-flight: target shape must be `Ssh`. The dispatcher
    // (`AttachDispatch::RunAttachSsh`) is the only production caller
    // and only constructs this with `Ssh`, so the check is defensive
    // — a wrong shape never reaches here at runtime, and we exit
    // before any side effects (no SSH spawn, no Frontend, no eprintln
    // races).
    if !matches!(&target, AttachTarget::Ssh { .. }) {
        return Err(AttachError::Io(std::io::Error::other(
            "run_attach_ssh requires AttachTarget::Ssh",
        )));
    }

    // The Frontend lives in the outer scope so it persists across
    // reconnect attempts. Lazy-initialized on the first successful
    // handshake — until then, errors reach a cooked terminal
    // directly via main.rs's `eprintln!`. The slot's `Some`-ness
    // doubles as the `initial_handshake_complete` signal: once
    // Frontend is up, transient failures reconnect indefinitely
    // (mid-session disconnect should not bounce the user out).
    let mut frontend_slot: Option<Frontend> = None;
    let mut backoff = BackoffSchedule::new();
    let mut handshake_attempts: u32 = 0;
    let mut iteration: u32 = 0;

    let result: Result<(), AttachError> = loop {
        // Pre-attempt overlay (skipped on the very first iteration —
        // no Frontend yet, and "reconnecting" would lie about state).
        // The chosen banner depends on whether we're reconnecting
        // mid-session (Frontend is up) or still trying to land the
        // first handshake (Frontend never came up).
        if iteration > 0 {
            if let Some(f) = frontend_slot.as_mut() {
                let banner = banner_session_reconnect();
                let _ = f.draw_status_overlay(banner);
            } else {
                // Slot is None and we're past iteration 0 — only
                // reachable through `ReconnectHandshake`. Show the
                // bounded counter so the user knows how many tries
                // remain before we give up.
                //
                // No Frontend means no overlay; the cooked terminal
                // already saw the previous attempt's stderr. Fall
                // through to the attempt without painting.
            }
        }

        let initial_handshake_complete = frontend_slot.is_some();
        let verdict = attempt_session(&mut frontend_slot, &target, initial_handshake_complete);

        match verdict {
            ReconnectVerdict::ExitClean => {
                if let Some(f) = frontend_slot.as_mut() {
                    let _ = f.clear_status_overlay();
                }
                break Ok(());
            }
            ReconnectVerdict::ExitProtocolError { error }
            | ReconnectVerdict::ExitPolicy { error } => {
                break Err(error);
            }
            ReconnectVerdict::Reconnect { error: _last_error } => {
                // Mid-session disconnect: indefinite retry. The
                // backoff schedule is intentionally NOT reset between
                // failed attempts — it walks the curve and plateaus
                // at 30s, matching mosh. Reset on a sustained
                // re-pump is a v0.3 refinement (we'd need a
                // "pump_ran_for_at_least_N_seconds" signal that
                // doesn't exist yet).
                let delay = backoff.next_delay();
                match sleep_with_countdown(frontend_slot.as_mut(), delay) {
                    SleepOutcome::Cancelled => break Ok(()),
                    SleepOutcome::Elapsed => {}
                }
            }
            ReconnectVerdict::ReconnectHandshake { error } => {
                handshake_attempts = handshake_attempts.saturating_add(1);
                if handshake_attempts >= HANDSHAKE_RETRY_CAP {
                    // Exhausted the handshake retry budget. Surface
                    // the most recent error; the SSH stderr tail
                    // (if any) is already inside it via
                    // `AttachError::SshChildExited`.
                    break Err(error);
                }
                // Paint the bounded counter for the next attempt.
                // Frontend may or may not be up — paint only if it
                // is. (See note above: handshake-reconnect implies
                // slot is None, but we re-check in case future
                // refactors loosen the invariant.)
                if let Some(f) = frontend_slot.as_mut() {
                    let banner = banner_handshake_reconnect(
                        handshake_attempts.saturating_add(1),
                        HANDSHAKE_RETRY_CAP,
                    );
                    let _ = f.draw_status_overlay(&banner);
                }
                let delay = backoff.next_delay();
                match sleep_with_countdown(frontend_slot.as_mut(), delay) {
                    SleepOutcome::Cancelled => break Ok(()),
                    SleepOutcome::Elapsed => {}
                }
            }
        }

        iteration = iteration.saturating_add(1);
    };

    // Tear down raw mode (if engaged) before main.rs's eprintln.
    drop(frontend_slot);

    result
}

/// Banner painted while a session-reconnect attempt is in flight.
///
/// Used when the Frontend is already up (the user has seen the
/// editor render at least once on this `run_attach_ssh` invocation).
/// No counter — session reconnects are unbounded.
fn banner_session_reconnect() -> &'static str {
    "[pmacs reconnecting...]"
}

/// Banner painted while a handshake-reconnect attempt is in flight.
///
/// Shows the bounded counter (`attempt N of M`) so the user knows
/// when to stop waiting and investigate. `attempt` is the 1-based
/// index of the next attempt about to fire; `cap` is
/// [`HANDSHAKE_RETRY_CAP`].
fn banner_handshake_reconnect(attempt: u32, cap: u32) -> String {
    format!("[pmacs reconnecting (attempt {attempt} of {cap})...]")
}

/// Banner painted while sleeping between attempts.
///
/// Shows a per-second countdown so the user can predict the next
/// retry. Rounds remaining time UP so the displayed value matches
/// "you'll see at most this much delay" — a sub-second leftover at
/// the bottom of the wait shows as `1s`, not `0s`, until it actually
/// elapses.
fn banner_disconnected_countdown(remaining: Duration) -> String {
    // Round up: 4001ms → 5s, 4000ms → 4s, 1ms → 1s.
    let secs = remaining.as_secs() + u64::from(remaining.subsec_millis() > 0);
    if secs == 0 {
        "[pmacs disconnected — reconnecting now — Ctrl-C to exit]".to_string()
    } else {
        format!("[pmacs disconnected — reconnecting in {secs}s — Ctrl-C to exit]")
    }
}

/// Outcome of [`sleep_with_countdown`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SleepOutcome {
    /// User pressed Ctrl-C during the wait. The reconnect loop
    /// should exit cleanly (no error message — the user knows they
    /// canceled).
    Cancelled,
    /// The full delay elapsed without a cancellation. The loop
    /// should proceed with the next attempt.
    Elapsed,
}

/// Sleep for `total` while painting a per-second countdown banner.
///
/// Uses [`Frontend::poll_event`] as the wait primitive so the loop
/// can react to Ctrl-C immediately (raw mode delivers Ctrl-C as a
/// `KeyEvent`, not as `SIGINT` — terminal `ISIG` is disabled by
/// `enable_raw_mode`). Other events (resize, mouse, F-keys) are
/// drained but ignored; resize updates `Frontend::size` as a side
/// effect of `poll_event`, which is the exact behavior the next
/// `attempt_session` needs for its `initial_size`.
///
/// If `frontend` is `None` (handshake-reconnect with no Frontend up
/// yet), the function falls back to a plain `thread::sleep` and
/// cannot be cancelled — the user has nothing to interact with
/// anyway, and the cooked-terminal stderr already shows the SSH
/// error.
fn sleep_with_countdown(frontend: Option<&mut Frontend>, total: Duration) -> SleepOutcome {
    let Some(f) = frontend else {
        // No Frontend — no overlay, no input pump. Plain sleep.
        thread::sleep(total);
        return SleepOutcome::Elapsed;
    };

    let deadline = Instant::now() + total;

    // Initial paint so the banner appears immediately, not after the
    // first poll tick.
    let initial_banner = banner_disconnected_countdown(total);
    let _ = f.draw_status_overlay(&initial_banner);
    let mut last_painted_secs: Option<u64> =
        Some(total.as_secs() + u64::from(total.subsec_millis() > 0));

    loop {
        let now = Instant::now();
        if now >= deadline {
            return SleepOutcome::Elapsed;
        }
        let remaining = deadline - now;
        let secs = remaining.as_secs() + u64::from(remaining.subsec_millis() > 0);
        if last_painted_secs != Some(secs) {
            let banner = banner_disconnected_countdown(remaining);
            let _ = f.draw_status_overlay(&banner);
            last_painted_secs = Some(secs);
        }

        // Poll for at most `remaining` and at most 250ms — short
        // enough that the countdown updates feel live, long enough
        // to avoid burning CPU in a tight poll loop.
        let tick = std::cmp::min(Duration::from_millis(250), remaining);
        match f.poll_event(tick) {
            Ok(Some(Event::Key(KeyEvent {
                code: KeyCode::Char('c'),
                modifiers,
                kind: KeyEventKind::Press,
                ..
            }))) if modifiers.contains(KeyModifiers::CONTROL) => {
                return SleepOutcome::Cancelled;
            }
            Ok(_) | Err(_) => {
                // Any other event (resize, mouse, key) → drain and
                // continue. `poll_event` already updated
                // `frontend.size` if it was a resize, which is what
                // we want for the next attempt's `initial_size`.
                // Errors from `poll_event` are extremely rare
                // (terminal disconnected); we treat them like
                // "tick elapsed" and keep waiting — if the terminal
                // is genuinely gone, the next attempt's pump will
                // surface the failure cleanly.
            }
        }
    }
}

/// One reconnect-aware attach attempt: spawn SSH, run the
/// `Hello`/`AttachRequest` handshake, lazy-init the Frontend on
/// first success, run the pump, reap the child, return a
/// [`ReconnectVerdict`].
///
/// `frontend_slot` is the outer loop's persistent slot. On entry:
/// * `None` → first attempt; the function will print the attach
///   info banner and call [`Frontend::new`] after the handshake.
/// * `Some(_)` → reconnect; no terminal teardown, no info banner,
///   the existing Frontend is reused so the cell grid stays on
///   screen behind the M5.8c status overlay.
///
/// Postcondition (on `Ok`-mapped verdicts): if the function got past
/// `Frontend::new`, `frontend_slot` is `Some`. Errors before that
/// point keep it `None`, preserving the cooked-terminal contract for
/// pre-Frontend failures.
///
/// Doc rule for M5.8: this function does NOT toggle terminal modes
/// or clear the screen. Frontend lifecycle is the outer scope's
/// responsibility, and the cell grid must persist across attempts so
/// the M5.8c overlay is visually layered on top of stable content.
fn attempt_session(
    frontend_slot: &mut Option<Frontend>,
    target: &AttachTarget,
    initial_handshake_complete: bool,
) -> ReconnectVerdict {
    let raw_result = run_one_session(frontend_slot, target);
    match raw_result {
        Ok(()) => ReconnectVerdict::ExitClean,
        Err(error) => classify_for_reconnect(error, initial_handshake_complete),
    }
}

/// Protocol byte source plus the SSH-stderr tee's join handle and
/// captured tail. Returned by [`open_protocol_channel`]; named so the
/// handshake's binding site isn't a four-line inline tuple type.
type ProtocolChannel = (
    Box<dyn Read + Send>,
    thread::JoinHandle<()>,
    Arc<Mutex<VecDeque<u8>>>,
);

/// Pick the SSH channel the protocol rides on.
///
/// `protocol_over_stderr` is [`SSH_PROTOCOL_DEFAULT`]-derived (F8b:
/// stderr is the default — see `SshProtocolChannel`). When stderr:
/// protocol reads SSH stderr, remote stderr diagnostics are disabled
/// by the caller, and there is no tee (a no-op join handle / empty
/// tail keep the return shape). When stdout (the F8b fallback /
/// override): protocol reads SSH stdout, and SSH stderr is tee'd to
/// our stderr only on the first attempt (`tee_to_stderr`) — on
/// reconnect raw mode is active and live tee'd bytes would corrupt
/// the cell grid, so the tail is still captured for the give-up
/// message but not echoed.
fn open_protocol_channel(
    child_stdout: std::process::ChildStdout,
    child_stderr: std::process::ChildStderr,
    protocol_over_stderr: bool,
    tee_to_stderr: bool,
) -> ProtocolChannel {
    if protocol_over_stderr {
        attach_debug("using SSH stderr as protocol stream; remote stderr diagnostics disabled");
        (
            Box::new(DebugReader::new("ssh stderr(protocol)", child_stderr, 4)),
            thread::spawn(|| {}),
            Arc::new(Mutex::new(VecDeque::new())),
        )
    } else {
        let (stderr_handle, stderr_tail) = spawn_stderr_tee(child_stderr, tee_to_stderr);
        (
            Box::new(DebugReader::new("ssh stdout", child_stdout, 4)),
            stderr_handle,
            stderr_tail,
        )
    }
}

/// Spawn the F8b watchdog: with attach-debug on, warn if the `Hello`
/// hasn't arrived within 5s (the F8b symptom — remote bridge
/// connected but no daemon bytes reached local stdout). Returns the
/// flag the caller sets once `Hello` is read or fails, silencing it.
fn spawn_hello_watchdog() -> Arc<AtomicBool> {
    let done = Arc::new(AtomicBool::new(false));
    if attach_debug_enabled() {
        let done = Arc::clone(&done);
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(5));
            if !done.load(Ordering::SeqCst) {
                eprintln!(
                    "pmacs attach debug: still waiting for Hello after 5s; \
                     remote bridge connected but no daemon bytes reached local stdout"
                );
            }
        });
    }
    done
}

/// Run the attach pump to completion, then reap the child and drain
/// the stderr tee with the Frontend still alive (none of these steps
/// emit terminal output, so raw mode being on is harmless). The
/// Frontend is NOT dropped — the outer scope owns its lifetime so it
/// persists into the next reconnect attempt.
fn run_pump_and_reap(
    mut child: Child,
    child_stdin: std::process::ChildStdin,
    protocol_reader: Box<dyn Read + Send>,
    stderr_handle: thread::JoinHandle<()>,
    stderr_tail: &Arc<Mutex<VecDeque<u8>>>,
    frontend: &mut Frontend,
    assigned_frontend_id: FrontendId,
) -> Result<(), AttachError> {
    let pid = child.id();
    let io = AttachIo {
        reader: protocol_reader,
        writer: Box::new(child_stdin),
        kick: ssh_kick(pid),
    };
    let pump_result = run_attach_pair(io, frontend, assigned_frontend_id);
    attach_debug(format!(
        "attach pump exited: {:?}",
        pump_result.as_ref().err()
    ));
    let exit_status = child.wait().ok();
    let _ = stderr_handle.join();
    let stderr_text = drain_stderr_tail(stderr_tail);
    classify_ssh_exit(pump_result, exit_status, stderr_text)
}

/// Body of [`attempt_session`] expressed in raw `AttachError` so the
/// caller can fold the result through [`classify_for_reconnect`].
/// Splitting this out keeps the IO path linear and tests-friendly:
/// `attempt_session` is just classify + dispatch.
// The protocol-channel, Hello-watchdog, and pump-and-reap phases are
// already factored out (`open_protocol_channel`, `spawn_hello_watchdog`,
// `run_pump_and_reap`). What remains is a linear Hello/AttachRequest
// handshake whose several early-return error paths each need ownership
// of `child` + the stderr handles to tear down via
// `handshake_error_with_child`; extracting it further would fragment
// that error-ownership flow rather than clarify it. Same precedent as
// the `#[allow(clippy::too_many_lines)]` on `run_attach_pair` above.
#[allow(clippy::too_many_lines)]
fn run_one_session(
    frontend_slot: &mut Option<Frontend>,
    target: &AttachTarget,
) -> Result<(), AttachError> {
    let mut cmd = build_ssh_command(target).expect("target shape pre-validated by caller");
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let bin_for_error = ssh_binary();
    attach_debug(format!("spawning ssh command: {cmd:?}"));
    let mut child = cmd.spawn().map_err(|source| AttachError::SshSpawnFailed {
        command: bin_for_error,
        source,
    })?;
    attach_debug(format!("ssh child spawned with pid {}", child.id()));

    let child_stdout = child
        .stdout
        .take()
        .expect("Stdio::piped on stdout guarantees a handle");
    let mut child_stdin = child
        .stdin
        .take()
        .expect("Stdio::piped on stdin guarantees a handle");
    let child_stderr = child
        .stderr
        .take()
        .expect("Stdio::piped on stderr guarantees a handle");

    let protocol_over_stderr = ssh_protocol_channel() == SshProtocolChannel::Stderr;
    let (mut protocol_reader, stderr_handle, stderr_tail) = open_protocol_channel(
        child_stdout,
        child_stderr,
        protocol_over_stderr,
        frontend_slot.is_none(),
    );

    // Hello / AttachRequest handshake. On the FIRST attempt
    // (`frontend_slot` is `None`) raw mode is not engaged yet, so
    // failures here reach a cooked terminal via main.rs's eprintln.
    // On reconnect (`Some`) the Frontend is up and the M5.8c
    // overlay communicates state; handshake errors here are bubbled
    // up so the M5.8d loop can decide whether to retry.
    attach_debug("waiting for Hello from remote daemon bridge");
    let hello_wait_done = spawn_hello_watchdog();
    let hello: Hello = match read_message(&mut protocol_reader) {
        Ok(h) => h,
        Err(e) => {
            hello_wait_done.store(true, Ordering::SeqCst);
            attach_debug(format!("failed reading Hello: {e}"));
            return Err(handshake_error_with_child(
                child,
                stderr_handle,
                stderr_tail,
                e.into(),
            ));
        }
    };
    hello_wait_done.store(true, Ordering::SeqCst);
    attach_debug(format!(
        "received Hello: protocol_version={}, assigned_frontend_id={}",
        hello.protocol_version, hello.assigned_frontend_id.0
    ));
    // T M10.5: relaxed to range membership per
    // `§sec:m10-backward-compat`. Symmetric with the local-socket
    // attach path above.
    if !crate::protocol::is_supported_protocol_version(hello.protocol_version) {
        return Err(handshake_error_with_child(
            child,
            stderr_handle,
            stderr_tail,
            AttachError::VersionMismatch {
                server: hello.protocol_version,
                client: PROTOCOL_VERSION,
            },
        ));
    }

    // Determine initial_size. On reconnect, use the Frontend's
    // already-known size (it tracks resizes). On first attempt, query
    // crossterm directly because the Frontend isn't up yet.
    let initial_size = match frontend_slot.as_ref() {
        Some(f) => f.size(),
        None => match crossterm::terminal::size() {
            Ok((c, r)) => CellSize::new(u32::from(r), u32::from(c)),
            Err(e) => {
                return Err(handshake_error_with_child(
                    child,
                    stderr_handle,
                    stderr_tail,
                    AttachError::Terminal(e),
                ));
            }
        },
    };

    // T M10.5: match the server's protocol version so v1.0 frontends
    // attaching to v0.1 daemons advertise protocol_version=1. Same
    // pattern as the local-socket path above.
    let req = AttachRequest {
        protocol_version: hello.protocol_version,
        frontend_capabilities: build_capabilities(),
        initial_size,
    };
    if let Err(e) = write_message(&mut child_stdin, &req) {
        attach_debug(format!("failed writing AttachRequest: {e}"));
        return Err(handshake_error_with_child(
            child,
            stderr_handle,
            stderr_tail,
            e.into(),
        ));
    }
    attach_debug("sent AttachRequest");

    // First-attempt-only side effects. On reconnect the user already
    // saw the info banner and the Frontend is already up; we skip
    // both so the reconnect is visually a no-op except for the
    // overlay clearing.
    if frontend_slot.is_none() {
        print_attach_info(&hello);
        match Frontend::new() {
            Ok(f) => *frontend_slot = Some(f),
            Err(e) => {
                return Err(handshake_error_with_child(
                    child,
                    stderr_handle,
                    stderr_tail,
                    AttachError::Terminal(e),
                ));
            }
        }
    }
    let frontend = frontend_slot
        .as_mut()
        .expect("frontend_slot was just initialized or already Some");

    // The frontend now owns the terminal (raw mode + alt-screen).
    // From here, including any later reconnect handshake, breadcrumbs
    // go to the log file only — stderr is the screen the renderer
    // paints.
    mark_tui_terminal_owned();

    run_pump_and_reap(
        child,
        child_stdin,
        protocol_reader,
        stderr_handle,
        &stderr_tail,
        frontend,
        hello.assigned_frontend_id,
    )
}

/// Helper for the early-exit paths in [`run_attach_ssh`] that want
/// to surface a handshake error: reaps the child, joins the stderr
/// tee, returns `original_error` (raw mode never engaged, so the
/// error reaches a normal terminal directly).
fn handshake_error_with_child(
    mut child: Child,
    stderr_handle: thread::JoinHandle<()>,
    _stderr_tail: Arc<Mutex<VecDeque<u8>>>,
    original_error: AttachError,
) -> AttachError {
    // Best-effort tear-down of the child. SIGTERM unsticks any read
    // it might be doing on its end of our broken handshake; wait
    // reaps. Errors are ignored — if the child has already exited
    // (e.g., remote `pmacs` not on PATH), `kill` returns ESRCH and
    // we move on.
    #[allow(clippy::cast_possible_wrap)]
    let pid = nix::unistd::Pid::from_raw(child.id() as i32);
    let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM);
    let _ = child.wait();
    let _ = stderr_handle.join();
    original_error
}

/// Drain the stderr tail ring buffer into an owned `String`,
/// lossily decoding non-UTF-8 bytes (the user's terminal already
/// rendered the original bytes; the string is just for the
/// diagnostic message).
fn drain_stderr_tail(tail: &Arc<Mutex<VecDeque<u8>>>) -> String {
    let bytes: Vec<u8> = tail.lock().expect("tail mutex").iter().copied().collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Map (pump result, child exit status, stderr tail) → final
/// `AttachError` or success.
///
/// * Pump returned `Err` → propagate it; the child's exit is
///   noise compared to the pump's failure.
/// * Pump returned `Ok` and child exited 0 → `Ok(())`.
/// * Pump returned `Ok` and child exited non-zero → classify into
///   [`AttachError::SshChildExited`] with the captured stderr tail.
/// * Pump returned `Ok` and child status unknown (wait failed) →
///   treat as success; the bridge ran to completion as far as we
///   could tell.
fn classify_ssh_exit(
    pump_result: Result<(), AttachError>,
    exit_status: Option<std::process::ExitStatus>,
    stderr_tail: String,
) -> Result<(), AttachError> {
    pump_result?;
    match exit_status {
        Some(s) if s.success() => Ok(()),
        Some(s) => Err(AttachError::SshChildExited {
            code: s.code(),
            stderr_tail,
        }),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Try to bind a `UnixListener` at `path`. On `PermissionDenied`
    /// (e.g., a sandboxed CI environment that disallows `AF_UNIX`
    /// socket creation), prints a skip notice and returns `None`;
    /// the calling test should early-return so the suite reports
    /// `0 failed` rather than a misleading panic. Mirror of the
    /// helper in `daemon_attach.rs`'s test module.
    fn bind_or_skip(path: &std::path::Path) -> Option<UnixListener> {
        match UnixListener::bind(path) {
            Ok(l) => Some(l),
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                eprintln!(
                    "test skipped: UnixListener::bind {} → PermissionDenied \
                     (sandboxed environment).",
                    path.display()
                );
                None
            }
            Err(e) => panic!("UnixListener::bind {} failed: {e}", path.display()),
        }
    }

    #[test]
    fn format_uptime_shapes() {
        assert_eq!(format_uptime(5), "5s");
        assert_eq!(format_uptime(59), "59s");
        assert_eq!(format_uptime(60), "1m0s");
        assert_eq!(format_uptime(125), "2m5s");
        assert_eq!(format_uptime(3599), "59m59s");
        assert_eq!(format_uptime(3600), "1h0m");
        assert_eq!(format_uptime(7325), "2h2m");
    }

    #[test]
    fn detach_key_recognized() {
        use crossterm::event::{KeyEvent as CtKey, KeyEventState, KeyModifiers as CtMods};
        let f12 = Event::Key(CtKey {
            code: KeyCode::F(12),
            modifiers: CtMods::empty(),
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        });
        assert!(is_detach_key(&f12));

        let f11 = Event::Key(CtKey {
            code: KeyCode::F(11),
            modifiers: CtMods::empty(),
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        });
        assert!(!is_detach_key(&f11));

        let f12_release = Event::Key(CtKey {
            code: KeyCode::F(12),
            modifiers: CtMods::empty(),
            kind: KeyEventKind::Release,
            state: KeyEventState::empty(),
        });
        assert!(!is_detach_key(&f12_release));
    }

    #[test]
    fn build_capabilities_advertises_all_v01_features() {
        let c = build_capabilities();
        assert!(c.synchronized_output);
        assert!(c.unicode_smp);
        assert!(c.true_color);
        assert!(c.mouse);
        assert!(c.bracketed_paste);
        // terminal_kind depends on TERM env var; not asserted.
    }

    // -----------------------------------------------------------------
    // M5.7a — pump generalization tests
    // -----------------------------------------------------------------
    //
    // These tests drive `run_attach_pair` end-to-end without taking
    // over a real terminal. They use:
    //
    // * `UnixStream::pair()` for an in-process socket pair (one end
    //   is the "daemon," the other is the AttachIo).
    // * `FakeFrontend`, a tiny `AttachPumpFrontend` impl that records
    //   `present_messages` calls and never produces input.
    //
    // The tests cover four contracts:
    //
    // 1. Pump routes instance messages to the frontend (test 2).
    // 2. `kick` fires on a clean Goodbye exit (test 3a).
    // 3. `kick` fires on an error exit (test 3b).
    // 4. `kick` actually wakes a blocked reader within bounded time
    //    (test 4).
    // 5. Hello/AttachRequest handshake errors surface at the
    //    construction site, not from inside the pump (test 5).

    use crate::protocol::{
        GoodbyeReason, Hello, InstanceCapabilities, InstanceIdentity, PROTOCOL_VERSION,
    };
    use crate::transport::write_message;
    use std::os::unix::net::UnixListener;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Records `present_messages` calls; never produces input.
    ///
    /// `present_returns_error = true` makes the next `present_messages`
    /// call fail, which forces the pump into the error-exit path so
    /// tests can observe `kick` firing under both clean and error
    /// terminations.
    #[derive(Default)]
    struct FakeFrontend {
        presented: Vec<InstanceMessage>,
        present_returns_error: bool,
    }

    impl AttachPumpFrontend for FakeFrontend {
        fn present_messages(&mut self, msgs: &[InstanceMessage]) -> std::io::Result<()> {
            if self.present_returns_error {
                return Err(std::io::Error::other("test forced present error"));
            }
            self.presented.extend(msgs.iter().cloned());
            Ok(())
        }
        fn poll_event(&mut self, _timeout: Duration) -> std::io::Result<Option<Event>> {
            // Tests drive the daemon side; no local input is generated.
            Ok(None)
        }
        fn size(&self) -> CellSize {
            CellSize::new(24, 80)
        }
    }

    /// Test-only `Read` wrapper that flips an `AtomicBool` whenever
    /// its inner reader is called. Used to synchronize the test
    /// against "the reader thread has entered its read call" without
    /// resorting to wall-clock sleeps.
    ///
    /// External wrapper by design: production types stay free of
    /// test-only hooks. The signal fires on every `read` call (not
    /// just the first); the test only cares about observing it
    /// transition once, so cheap repeated stores are harmless.
    struct EnteredReadSignaler<R: Read> {
        inner: R,
        entered: Arc<AtomicBool>,
    }

    impl<R: Read> Read for EnteredReadSignaler<R> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.entered.store(true, Ordering::Release);
            self.inner.read(buf)
        }
    }

    /// Build an `AttachIo` with a kick that increments `counter`.
    fn pipe_io_with_counting_kick(socket: UnixStream, counter: Arc<AtomicUsize>) -> AttachIo {
        let reader = socket.try_clone().expect("try_clone reader");
        AttachIo {
            reader: Box::new(reader),
            writer: Box::new(socket),
            kick: Box::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
        }
    }

    #[test]
    fn pump_routes_instance_messages_to_frontend() {
        let (daemon_side, frontend_side) = UnixStream::pair().expect("UnixStream::pair");
        let io = build_local_socket_io(frontend_side).expect("build_local_socket_io");

        let daemon = thread::spawn(move || {
            let mut w = daemon_side;
            write_message(&mut w, &InstanceMessage::Cursor(None)).unwrap();
            // Brief gap so the pump definitely processes Cursor before
            // the Goodbye-exit path latches; not load-bearing for
            // correctness (the drain loop handles either order) but
            // makes the test cover the typical sequencing.
            thread::sleep(Duration::from_millis(50));
            write_message(
                &mut w,
                &InstanceMessage::Goodbye(GoodbyeReason::ShuttingDown),
            )
            .unwrap();
        });

        let mut frontend = FakeFrontend::default();
        let result = run_attach_pair(io, &mut frontend, FrontendId::LOCAL);

        daemon.join().expect("daemon thread");

        assert!(result.is_ok(), "pump exits Ok on Goodbye: got {result:?}");
        assert_eq!(frontend.presented.len(), 1, "exactly one message presented");
        assert!(matches!(
            frontend.presented[0],
            InstanceMessage::Cursor(None)
        ));
    }

    #[test]
    fn kick_fires_on_clean_goodbye_exit() {
        let (daemon_side, frontend_side) = UnixStream::pair().expect("UnixStream::pair");
        let counter = Arc::new(AtomicUsize::new(0));
        let io = pipe_io_with_counting_kick(frontend_side, counter.clone());

        let daemon = thread::spawn(move || {
            let mut w = daemon_side;
            write_message(
                &mut w,
                &InstanceMessage::Goodbye(GoodbyeReason::ShuttingDown),
            )
            .unwrap();
        });

        let mut frontend = FakeFrontend::default();
        let result = run_attach_pair(io, &mut frontend, FrontendId::LOCAL);

        daemon.join().expect("daemon thread");

        assert!(result.is_ok());
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "kick must fire exactly once on clean exit",
        );
    }

    #[test]
    fn kick_fires_on_present_error_exit() {
        let (daemon_side, frontend_side) = UnixStream::pair().expect("UnixStream::pair");
        let counter = Arc::new(AtomicUsize::new(0));
        let io = pipe_io_with_counting_kick(frontend_side, counter.clone());

        let daemon = thread::spawn(move || {
            let mut w = daemon_side;
            write_message(&mut w, &InstanceMessage::Cursor(None)).unwrap();
            // Drop daemon_side here. The pump will either hit the
            // present-error path first (return Err) or the EOF path
            // first (return Ok). The kick must fire either way; the
            // assertion is on the kick count, not the exit reason.
        });

        let mut frontend = FakeFrontend {
            presented: Vec::new(),
            present_returns_error: true,
        };
        let result = run_attach_pair(io, &mut frontend, FrontendId::LOCAL);

        daemon.join().expect("daemon thread");

        // The race between "Cursor reaches the pump and present errors"
        // and "daemon socket closes and reader signals disconnect" is
        // not deterministic in a unit test. What IS deterministic: kick
        // fires exactly once, regardless of which path the pump took.
        let _ = result; // either Ok (clean EOF) or Err(Terminal) (present errored)
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "kick must fire exactly once on any exit path",
        );
    }

    #[test]
    fn kick_wakes_blocked_reader() {
        // What this test asserts: after `kick()` fires, the reader
        // thread terminates. That is, the kick mechanism wakes a
        // reader that would otherwise wait on the socket forever.
        //
        // What this test does NOT assert: a specific wake latency.
        // The 5s join bound is intentionally generous so that CI
        // scheduler jitter (heavily loaded VM hosts can stall threads
        // for hundreds of ms cumulatively) does not turn a correctness
        // test into a flake. The steady-state wake budget per
        // `KickAwareUnixReader`'s contract is ~10ms (one poll cycle),
        // but observing that bound under timing pressure is not what
        // this test is for. **Do not tighten the 5s bound back toward
        // 1s on the grounds that 5s is much larger than the
        // steady-state budget** — the steady-state budget is not what
        // is being tested. A real bug (kick mechanism is broken, the
        // reader runs forever) hits this bound; CI jitter does not
        // come close.
        //
        // Synchronization: rather than guessing how long the reader
        // thread takes to start with `thread::sleep`, the test wraps
        // the reader in an `EnteredReadSignaler` that flips an
        // `AtomicBool` when the inner reader is first called. The
        // test spins on that flag (bounded) so kick fires only once
        // we know the reader is actively reading from the socket.
        // No wall-clock guesses; no test-only paths in production
        // types.

        // Hold the daemon side so the kick is the only thing that can
        // wake the reader. If we let the daemon side close, the reader
        // sees EOF naturally and we'd be testing nothing.
        let (_daemon_side, frontend_side) = UnixStream::pair().expect("UnixStream::pair");
        let io = build_local_socket_io(frontend_side).expect("build_local_socket_io");
        let AttachIo {
            reader,
            writer: _writer,
            kick,
        } = io;

        let entered = Arc::new(AtomicBool::new(false));
        let signaling_reader: Box<dyn Read + Send> = Box::new(EnteredReadSignaler {
            inner: reader,
            entered: Arc::clone(&entered),
        });

        let (tx, _rx) = mpsc::channel::<InstanceMessage>();
        let reader_handle = thread::spawn(move || run_reader(signaling_reader, tx));

        // Wait for the reader to enter its read call. Bounded so a
        // never-spawning reader fails the test instead of hanging.
        let entry_deadline = Instant::now() + Duration::from_secs(1);
        while !entered.load(Ordering::Acquire) {
            assert!(
                Instant::now() < entry_deadline,
                "reader thread did not enter its read call within 1s",
            );
            thread::sleep(Duration::from_millis(1));
        }

        kick();

        // Bound the join: spawn a watcher that joins and signals via
        // a channel, then recv_timeout against the channel.
        let (done_tx, done_rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = reader_handle.join();
            let _ = done_tx.send(());
        });
        done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("reader thread must exit within 5s after kick");
    }

    #[test]
    fn version_mismatch_errors_at_construction_site() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let socket_path = tmp.path().join("test.sock");
        let Some(listener) = bind_or_skip(&socket_path) else {
            return;
        };

        // Fake daemon: accept one connection, write a Hello with a
        // bogus protocol version, exit. The accept blocks until
        // run_attach connects.
        let server = thread::spawn(move || {
            let (mut stream, _addr) = listener.accept().expect("accept");
            let bad_hello = Hello {
                protocol_version: PROTOCOL_VERSION + 999,
                assigned_frontend_id: FrontendId::LOCAL,
                instance_identity: InstanceIdentity::for_running_process(
                    None,
                    std::time::Instant::now(),
                ),
                instance_capabilities: InstanceCapabilities::default(),
            };
            write_message(&mut stream, &bad_hello).expect("write Hello");
        });

        let result = run_attach(socket_path);
        server.join().expect("server thread");

        // The handshake check happens BEFORE Frontend::new, so the
        // error reaches us without raw mode ever engaging. The
        // boundary contract: pump-related errors never wear the
        // VersionMismatch shape; this shape can only come from the
        // construction-site handshake.
        match result {
            Err(AttachError::VersionMismatch {
                server: srv,
                client,
            }) => {
                assert_eq!(srv, PROTOCOL_VERSION + 999);
                assert_eq!(client, PROTOCOL_VERSION);
            }
            other => panic!("expected VersionMismatch, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // M5.7e — SSH transport tests
    // -----------------------------------------------------------------
    //
    // These tests cover the unit-testable surface of the SSH path:
    //
    // * `build_ssh_command` — argument construction for various
    //   `AttachTarget::Ssh` shapes, including the `PMACS_TEST_SSH_BIN`
    //   substitution.
    // * Error message content — exit-127 names "command not found";
    //   generic non-zero hints at scrollback; the spawn error names
    //   the binary path.
    //
    // End-to-end SSH activation (real subprocess via PMACS_TEST_SSH_BIN
    // pointing at a daemon-attach helper) lives in
    // `tests/m5_7_acceptance.rs` (M5.7g).

    use std::ffi::OsStr;

    /// Extract `Command`'s arg list as a borrowed `&OsStr` vector.
    /// `Command::get_args` returns the args sans program; we read the
    /// program separately via `Command::get_program`.
    fn ssh_args(cmd: &Command) -> Vec<&OsStr> {
        cmd.get_args().collect()
    }

    #[test]
    fn build_ssh_command_for_bare_host() {
        // SAFETY: env mutation is only safe when no other thread is
        // touching env. Cargo's default test runner spawns threads,
        // but `PMACS_TEST_SSH_BIN` is read only by `ssh_binary` which
        // is called once per `build_ssh_command`. We avoid the race
        // by *not* mutating env here — the bare-host test asserts the
        // default binary is `ssh`.
        let target = AttachTarget::Ssh {
            host: "mac-studio".into(),
            user: None,
            instance_name: None,
        };
        let cmd = build_ssh_command(&target).expect("ssh target");
        // Default binary is `ssh` (no env override).
        // Note: we don't assert the exact program when the env var
        // might leak from a parallel test; assert only the args.
        let args = ssh_args(&cmd);
        let arg_strs: Vec<&str> = args.iter().filter_map(|s| s.to_str()).collect();
        assert_eq!(
            arg_strs,
            vec![
                "-T",
                "mac-studio",
                "env",
                "PMACS_ATTACH_PROTOCOL_FD=2",
                "pmacs",
                "--daemon-attach",
            ],
            "bare host, F8b stderr default: -T <host> env \
             PMACS_ATTACH_PROTOCOL_FD=2 pmacs --daemon-attach",
        );
    }

    #[test]
    fn build_ssh_command_with_user_emits_dash_l() {
        let target = AttachTarget::Ssh {
            host: "workstation".into(),
            user: Some("alice".into()),
            instance_name: None,
        };
        let cmd = build_ssh_command(&target).expect("ssh target");
        let arg_strs: Vec<&str> = ssh_args(&cmd).iter().filter_map(|s| s.to_str()).collect();
        assert_eq!(
            arg_strs,
            vec![
                "-T",
                "-l",
                "alice",
                "workstation",
                "env",
                "PMACS_ATTACH_PROTOCOL_FD=2",
                "pmacs",
                "--daemon-attach",
            ],
        );
    }

    #[test]
    fn build_ssh_command_with_instance_name_passes_through_socket_arg() {
        let target = AttachTarget::Ssh {
            host: "workstation".into(),
            user: Some("bob".into()),
            instance_name: Some("research".into()),
        };
        let cmd = build_ssh_command(&target).expect("ssh target");
        let arg_strs: Vec<&str> = ssh_args(&cmd).iter().filter_map(|s| s.to_str()).collect();
        assert_eq!(
            arg_strs,
            vec![
                "-T",
                "-l",
                "bob",
                "workstation",
                "env",
                "PMACS_ATTACH_PROTOCOL_FD=2",
                "pmacs",
                "--daemon-attach",
                "--socket",
                "research",
            ],
        );
    }

    #[test]
    fn build_ssh_command_returns_none_for_non_ssh_target() {
        // Defensive — the function is documented to return None for
        // non-SSH targets so misuse fails loudly at the call site
        // instead of silently constructing a malformed command.
        assert!(
            build_ssh_command(&AttachTarget::LocalSocket(PathBuf::from("/tmp/x.sock"))).is_none(),
        );
    }

    // F8b: pure channel-resolver tests. No env mutation (the resolver
    // core is env-free by design, so this is race-free under cargo's
    // threaded runner). Precedence: explicit > legacy > default.
    #[test]
    fn ssh_protocol_resolution_precedence_and_default() {
        use SshProtocolChannel::{Stderr, Stdout};

        // Default applies when nothing is set. The shipped default is
        // stderr (F8b); assert via the constant so flipping the one
        // switch keeps this test honest rather than brittle.
        assert_eq!(resolve_ssh_protocol(None, None, Stdout), Stdout);
        assert_eq!(resolve_ssh_protocol(None, None, Stderr), Stderr);
        assert_eq!(
            resolve_ssh_protocol(None, None, SSH_PROTOCOL_DEFAULT),
            Stderr
        );

        // Explicit override wins, case/space-insensitive, accepts fd
        // numbers too.
        for s in ["stdout", "STDOUT", "  Stdout ", "1"] {
            assert_eq!(resolve_ssh_protocol(Some(s), None, Stderr), Stdout, "{s:?}");
        }
        for s in ["stderr", "STDERR", "2"] {
            assert_eq!(resolve_ssh_protocol(Some(s), None, Stdout), Stderr, "{s:?}");
        }

        // Unrecognized explicit → ignored, falls through to legacy
        // then default.
        assert_eq!(resolve_ssh_protocol(Some("bogus"), None, Stderr), Stderr);
        assert_eq!(
            resolve_ssh_protocol(Some("bogus"), Some("0"), Stderr),
            Stdout
        );

        // Legacy back-compat: =0 → stdout, anything else non-empty →
        // stderr; empty → ignored.
        assert_eq!(resolve_ssh_protocol(None, Some("0"), Stderr), Stdout);
        assert_eq!(resolve_ssh_protocol(None, Some("1"), Stdout), Stderr);
        assert_eq!(resolve_ssh_protocol(None, Some("yes"), Stdout), Stderr);
        assert_eq!(resolve_ssh_protocol(None, Some(""), Stdout), Stdout);

        // Explicit beats legacy even when they disagree.
        assert_eq!(
            resolve_ssh_protocol(Some("stdout"), Some("1"), Stderr),
            Stdout
        );
        assert_eq!(
            resolve_ssh_protocol(Some("stderr"), Some("0"), Stdout),
            Stderr
        );

        // The remote fd mapping the bridge consumes.
        assert_eq!(Stdout.remote_fd(), 1);
        assert_eq!(Stderr.remote_fd(), 2);
    }

    #[test]
    fn ssh_child_exited_127_message_names_path_diagnostic() {
        let err = AttachError::SshChildExited {
            code: Some(127),
            stderr_tail: String::new(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("127"), "{msg}");
        assert!(msg.contains("command not found on remote"), "{msg}");
        assert!(msg.contains("which pmacs"), "{msg}");
    }

    #[test]
    fn ssh_child_exited_other_code_includes_scrollback_hint_when_stderr_nonempty() {
        let err = AttachError::SshChildExited {
            code: Some(255),
            stderr_tail: "Permission denied (publickey).\n".into(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("255"), "{msg}");
        assert!(msg.contains("scrollback"), "{msg}");
        assert!(msg.contains("Permission denied"), "{msg}");
    }

    #[test]
    fn ssh_child_exited_omits_scrollback_hint_when_stderr_empty() {
        let err = AttachError::SshChildExited {
            code: Some(1),
            stderr_tail: String::new(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("exited 1"), "{msg}");
        // No "scrollback" mention when there's nothing to point at.
        assert!(
            !msg.contains("scrollback"),
            "scrollback hint should be omitted on empty stderr: {msg}",
        );
    }

    #[test]
    fn ssh_child_terminated_by_signal_message() {
        let err = AttachError::SshChildExited {
            code: None,
            stderr_tail: String::new(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("terminated by signal"), "{msg}");
    }

    #[test]
    fn ssh_spawn_failed_message_names_command_and_workaround() {
        let err = AttachError::SshSpawnFailed {
            command: PathBuf::from("ssh"),
            source: std::io::Error::other("not found"),
        };
        let msg = format!("{err}");
        assert!(msg.contains("ssh"), "{msg}");
        assert!(msg.contains("not found"), "{msg}");
        assert!(
            msg.contains("PATH"),
            "spawn failure must hint at PATH: {msg}",
        );
    }

    #[test]
    fn classify_ssh_exit_zero_yields_ok() {
        // Synthesize an ExitStatus(0) via std's no-op subprocess
        // (`/bin/true` exits 0 on every Unix). We only need the
        // classification logic, not a real SSH session.
        let status = Command::new("true")
            .status()
            .expect("/bin/true should be runnable in tests");
        let result = classify_ssh_exit(Ok(()), Some(status), String::new());
        assert!(result.is_ok(), "exit 0 should classify as Ok: {result:?}");
    }

    #[test]
    fn classify_ssh_exit_nonzero_yields_ssh_child_exited() {
        let status = Command::new("false")
            .status()
            .expect("/bin/false should be runnable in tests");
        match classify_ssh_exit(Ok(()), Some(status), "scrollback".into()) {
            Err(AttachError::SshChildExited { code, stderr_tail }) => {
                assert_eq!(code, Some(1));
                assert_eq!(stderr_tail, "scrollback");
            }
            other => panic!("expected SshChildExited, got {other:?}"),
        }
    }

    #[test]
    fn classify_ssh_exit_propagates_pump_error_over_exit_status() {
        let status = Command::new("false").status().unwrap();
        let pump_err = AttachError::Terminal(std::io::Error::other("terminal busted"));
        match classify_ssh_exit(Err(pump_err), Some(status), String::new()) {
            Err(AttachError::Terminal(e)) => {
                assert!(format!("{e}").contains("terminal busted"));
            }
            other => panic!("pump error should win over exit status: got {other:?}"),
        }
    }

    #[test]
    fn classify_ssh_exit_no_status_treats_as_success() {
        // wait() failure: we have no exit status but the pump ran
        // cleanly. Treat as success — the bridge ran to completion
        // as far as we could tell.
        let result = classify_ssh_exit(Ok(()), None, String::new());
        assert!(
            result.is_ok(),
            "no status + Ok pump should be Ok: {result:?}"
        );
    }

    // -----------------------------------------------------------------
    // M5.8d — reconnect overlay banner formatters
    // -----------------------------------------------------------------

    #[test]
    fn banner_session_reconnect_is_static_text() {
        assert_eq!(banner_session_reconnect(), "[pmacs reconnecting...]");
    }

    #[test]
    fn banner_handshake_reconnect_includes_attempt_and_cap() {
        assert_eq!(
            banner_handshake_reconnect(2, 3),
            "[pmacs reconnecting (attempt 2 of 3)...]"
        );
        assert_eq!(
            banner_handshake_reconnect(1, HANDSHAKE_RETRY_CAP),
            format!("[pmacs reconnecting (attempt 1 of {HANDSHAKE_RETRY_CAP})...]")
        );
    }

    #[test]
    fn banner_countdown_rounds_up_subsecond_remainders() {
        // 4001ms → "5s" so the user never sees the displayed value
        // jitter below the actual remaining time.
        assert_eq!(
            banner_disconnected_countdown(Duration::from_millis(4001)),
            "[pmacs disconnected — reconnecting in 5s — Ctrl-C to exit]"
        );
        // Whole seconds round to themselves.
        assert_eq!(
            banner_disconnected_countdown(Duration::from_secs(4)),
            "[pmacs disconnected — reconnecting in 4s — Ctrl-C to exit]"
        );
        // 1ms still shows as 1s — sub-second leftovers count as a
        // whole second of remaining wait.
        assert_eq!(
            banner_disconnected_countdown(Duration::from_millis(1)),
            "[pmacs disconnected — reconnecting in 1s — Ctrl-C to exit]"
        );
    }

    #[test]
    fn banner_countdown_zero_remainder_says_now() {
        // True zero — the deadline arrived. Show "now" so the user
        // sees a meaningful tail before the next attempt fires.
        assert_eq!(
            banner_disconnected_countdown(Duration::ZERO),
            "[pmacs disconnected — reconnecting now — Ctrl-C to exit]"
        );
    }

    #[test]
    fn banner_countdown_full_30s_plateau() {
        // The schedule plateaus at 30s; verify the banner displays
        // that as the steady-state value, not e.g. "30s + epsilon".
        assert_eq!(
            banner_disconnected_countdown(Duration::from_secs(30)),
            "[pmacs disconnected — reconnecting in 30s — Ctrl-C to exit]"
        );
    }

    // T M10.7 — AttachError message formatting.
    //
    // Criterion 5 of the spec: the version-mismatch message must
    // tell the user which side is at the older version. These tests
    // pin the substring assertions explicitly so a future regression
    // (the message no longer naming the older side) fails visibly.

    #[test]
    fn version_mismatch_daemon_older_message_names_daemon() {
        let err = AttachError::VersionMismatch {
            server: 1,
            client: 2,
        };
        let msg = err.to_string();
        assert!(
            msg.contains("daemon is at the older version"),
            "criterion 5: message must name daemon as older when server < client; got: {msg}"
        );
    }

    #[test]
    fn version_mismatch_binary_older_message_names_binary() {
        let err = AttachError::VersionMismatch {
            server: 2,
            client: 1,
        };
        let msg = err.to_string();
        assert!(
            msg.contains("binary is at the older version"),
            "criterion 5: message must name client binary as older when server > client; got: {msg}"
        );
    }

    #[test]
    fn version_mismatch_equal_no_older_clause() {
        // Pathological case (the daemon shouldn't emit
        // VersionMismatch when versions match) — but the formatter
        // shouldn't claim an older side when there isn't one.
        let err = AttachError::VersionMismatch {
            server: 2,
            client: 2,
        };
        let msg = err.to_string();
        assert!(
            !msg.contains("older version"),
            "no older clause when equal; got: {msg}"
        );
    }

    #[test]
    fn capability_mismatch_message_names_multi_frontend() {
        // T M10.7 criterion 4 — the error names the specific
        // capability the frontend asked for that wasn't available.
        let err = AttachError::Rejected(GoodbyeReason::CapabilityMismatch {
            missing: vec!["multi_frontend".to_string()],
        });
        let msg = err.to_string();
        assert!(
            msg.contains("multi-frontend collaboration"),
            "criterion 4: message must name the capability in user-readable form; got: {msg}"
        );
    }

    #[test]
    fn capability_mismatch_message_names_crdt_replica() {
        let err = AttachError::Rejected(GoodbyeReason::CapabilityMismatch {
            missing: vec!["crdt_replica".to_string()],
        });
        let msg = err.to_string();
        assert!(
            msg.contains("CRDT replica participation"),
            "message must translate crdt_replica to user-readable form; got: {msg}"
        );
    }

    #[test]
    fn capability_mismatch_message_lists_multiple() {
        let err = AttachError::Rejected(GoodbyeReason::CapabilityMismatch {
            missing: vec!["multi_frontend".to_string(), "crdt_replica".to_string()],
        });
        let msg = err.to_string();
        assert!(msg.contains("multi-frontend collaboration"));
        assert!(msg.contains("CRDT replica participation"));
    }
}
