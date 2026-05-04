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
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
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
            Self::VersionMismatch { server, client } => write!(
                f,
                "protocol version mismatch (instance v{server}, client v{client})"
            ),
            Self::Rejected(reason) => write!(f, "instance rejected attach: {reason:?}"),
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
/// three ways and uses `shutdown(Read)` for the kick. The SSH
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
    /// Implementations may be destructive (SSH transport `SIGTERM`s
    /// the child) or non-destructive (local-socket transport calls
    /// `shutdown(Read)`). Callers do not distinguish — by the time
    /// the kick runs, the pump has already decided to exit.
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
    if hello.protocol_version != PROTOCOL_VERSION {
        return Err(AttachError::VersionMismatch {
            server: hello.protocol_version,
            client: PROTOCOL_VERSION,
        });
    }
    print_attach_info(&hello);

    let (cols, rows) = crossterm::terminal::size().map_err(AttachError::Terminal)?;
    let initial_size = CellSize::new(u32::from(rows), u32::from(cols));

    let req = AttachRequest {
        protocol_version: PROTOCOL_VERSION,
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
/// The kick clones a third handle for `shutdown(Read)`. Cloning may
/// fail (rare — the kernel is out of file descriptors), in which
/// case the caller propagates the error before raw mode engages.
fn build_local_socket_io(stream: UnixStream) -> Result<AttachIo, std::io::Error> {
    let reader = stream.try_clone()?;
    let kick_handle = stream.try_clone()?;
    Ok(AttachIo {
        reader: Box::new(reader),
        writer: Box::new(stream),
        kick: Box::new(move || {
            let _ = kick_handle.shutdown(Shutdown::Read);
        }),
    })
}

fn build_capabilities() -> FrontendCapabilities {
    // The v0.1 TUI implements all of these; we report them honestly
    // so the daemon doesn't strip features that work fine.
    FrontendCapabilities {
        synchronized_output: true,
        unicode_smp: true,
        true_color: true,
        mouse: true,
        bracketed_paste: true,
        terminal_kind: std::env::var("TERM").ok(),
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

    // Closure-and-call: any `return` from this closure still falls
    // through to `kick()` and `reader_handle.join()` below. Without
    // this wrapping a writer-side IO error inside the loop would skip
    // the wind-down.
    let result: Result<(), AttachError> = (|| {
        loop {
            // Drain instance messages. Goodbye exits immediately;
            // other messages are batched into a single
            // present_messages call.
            let mut batch: Vec<InstanceMessage> = Vec::new();
            let mut goodbye: Option<GoodbyeReason> = None;
            let mut reader_eof = false;
            loop {
                match rx.try_recv() {
                    Ok(InstanceMessage::Goodbye(reason)) => {
                        goodbye = Some(reason);
                        break;
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

            if let Err(e) = forward_event(&mut writer, &ev, assigned_id, frontend.size()) {
                // Likely a broken pipe — instance went away.
                eprintln!("pmacs: {e}");
                return Err(e);
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
    //    (per the kick contract). For local-socket: `shutdown(Read)`.
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
                .map(|d| u64::try_from(d.as_nanos()).unwrap_or(0))
                .unwrap_or(0);
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

/// Construct the [`Command`] for SSH attach without spawning.
///
/// Pure (modulo the env-var lookup for test substitution): used
/// directly by [`run_attach_ssh`] and by unit tests that want to
/// assert argument shape without spawning a real `ssh`.
///
/// Argument order: `[user-flag] host pmacs --daemon-attach
/// [--socket NAME]`. The user flag is `-l USER` if `user` is set;
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
    if let Some(u) = user {
        cmd.arg("-l").arg(u);
    }
    cmd.arg(host);
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

/// Body of [`attempt_session`] expressed in raw `AttachError` so the
/// caller can fold the result through [`classify_for_reconnect`].
/// Splitting this out keeps the IO path linear and tests-friendly:
/// `attempt_session` is just classify + dispatch.
fn run_one_session(
    frontend_slot: &mut Option<Frontend>,
    target: &AttachTarget,
) -> Result<(), AttachError> {
    let mut cmd = build_ssh_command(target).expect("target shape pre-validated by caller");
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let bin_for_error = ssh_binary();
    let mut child = cmd.spawn().map_err(|source| AttachError::SshSpawnFailed {
        command: bin_for_error,
        source,
    })?;

    let mut child_stdout = child
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

    // Tee SSH stderr to our stderr only on the first attempt, where
    // raw mode hasn't engaged yet. On reconnect attempts (slot is
    // `Some`), raw mode is active and live tee'd bytes would
    // corrupt the cell grid; the tail is still captured for the
    // give-up message via `AttachError::SshChildExited`.
    let tee_to_stderr = frontend_slot.is_none();
    let (stderr_handle, stderr_tail) = spawn_stderr_tee(child_stderr, tee_to_stderr);

    // Hello / AttachRequest handshake. On the FIRST attempt
    // (`frontend_slot` is `None`) raw mode is not engaged yet, so
    // failures here reach a cooked terminal via main.rs's eprintln.
    // On reconnect (`Some`) the Frontend is up and the M5.8c
    // overlay communicates state; handshake errors here are bubbled
    // up so the M5.8d loop can decide whether to retry.
    let hello: Hello = match read_message(&mut child_stdout) {
        Ok(h) => h,
        Err(e) => {
            return Err(handshake_error_with_child(
                child,
                stderr_handle,
                stderr_tail,
                e.into(),
            ));
        }
    };
    if hello.protocol_version != PROTOCOL_VERSION {
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

    let req = AttachRequest {
        protocol_version: PROTOCOL_VERSION,
        frontend_capabilities: build_capabilities(),
        initial_size,
    };
    if let Err(e) = write_message(&mut child_stdin, &req) {
        return Err(handshake_error_with_child(
            child,
            stderr_handle,
            stderr_tail,
            e.into(),
        ));
    }

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

    let pid = child.id();
    let io = AttachIo {
        reader: Box::new(child_stdout),
        writer: Box::new(child_stdin),
        kick: ssh_kick(pid),
    };

    let pump_result = run_attach_pair(io, frontend, hello.assigned_frontend_id);

    // Reap the child and drain the stderr tee with the Frontend
    // still alive. None of these steps emit terminal output, so raw
    // mode being on is harmless. Critically, we do NOT drop the
    // Frontend here — the outer scope owns its lifetime so it
    // persists into the next reconnect attempt.
    let exit_status = child.wait().ok();
    let _ = stderr_handle.join();
    let stderr_text = drain_stderr_tail(&stderr_tail);

    classify_ssh_exit(pump_result, exit_status, stderr_text)
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
    fn kick_wakes_blocked_reader_within_one_second() {
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

        let (tx, _rx) = mpsc::channel::<InstanceMessage>();
        let reader_handle = thread::spawn(move || run_reader(reader, tx));

        // Let the reader actually start blocking on its read.
        thread::sleep(Duration::from_millis(50));

        kick();

        // Bound the join: spawn a watcher that joins and signals via
        // a channel, then recv_timeout against the channel.
        let (done_tx, done_rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = reader_handle.join();
            let _ = done_tx.send(());
        });
        done_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("reader thread must exit within 1s after kick");
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
            vec!["mac-studio", "pmacs", "--daemon-attach"],
            "bare host should produce: <host> pmacs --daemon-attach",
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
            vec!["-l", "alice", "workstation", "pmacs", "--daemon-attach"],
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
                "-l",
                "bob",
                "workstation",
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
}
