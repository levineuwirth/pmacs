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
//! # A socket that ends with no `Goodbye`
//!
//! The daemon says `Goodbye` when it means to end a session. A socket that
//! ends without one was lost, which since E10r is what a frontend that stopped
//! reading past the daemon's bound finds when it is resumed, so both routes
//! attach again ([`run_attach`], [`run_attach_ssh`]), on one schedule and one set
//! of banners. A session comes back on a fresh view, not its own, and says why:
//! a grid session is minted no token (`docs/divergences.md`). A daemon that is
//! gone ends the local route, saying why once the alternate screen is left; the
//! SSH route's bridge starts a new one, which holds only what autosave kept, and the
//! notice says that when `Hello` tells it for certain and that it may be so when
//! `Hello` cannot ([`DaemonSeen`]), on either route.
//! [`HANDSHAKE_RETRY_CAP`] sessions in a row
//! that end before their first frame stop either route, so neither can spin, the
//! remote route's sentence naming where a daemon it started keeps its stderr
//! ([`FarSideStderr`]). A
//! reconnect over SSH may be asked for a passphrase, so its wait for `Hello` lends the
//! terminal and reads no key: `C-c` is a signal there and ends the route, and `F12`
//! does nothing ([`RECONNECT_HELLO_DEADLINE`]). A
//! frame cut short is the daemon's bound, and the frontend says so
//! ([`crate::transport::FRAME_CUT_SHORT_SENTENCE`]): no `Goodbye` can follow a
//! part-written frame.
//!
//! The F12 choice is tentative for v0.1; it conflicts with any user
//! keybind on F12, which we accept because F12 is rarely bound. A
//! later release will make it configurable.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{
    Arc, Condvar, Mutex, OnceLock,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread;
use std::time::Duration;

use std::time::Instant;

use nix::errno::Errno;
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};

use crate::attach_reconnect::{
    BackoffSchedule, DaemonSeen, DyingDaemons, HANDSHAKE_RETRY_CAP, ReconnectVerdict, Succession,
    classify_for_reconnect,
};
use crate::cell::CellSize;
use crate::frontend::{Event, Frontend, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crate::protocol::crossterm_translate::{key_from_crossterm, mouse_from_crossterm};
use crate::protocol::{
    AttachRequest, AttachTarget, FrontendCapabilities, FrontendEvent, FrontendId, GoodbyeReason,
    Hello, InstanceMessage, PROTOCOL_VERSION,
};
use crate::transport::{
    FRAME_CUT_SHORT_SENTENCE, MAX_FRAME_BYTES, TransportError, read_message, write_message,
};

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

/// How long an `ssh` has after the kick's SIGTERM before it has a SIGKILL. A real `ssh`
/// dies on the SIGTERM; a wrapper around one, or anything else in its place, may not.
const SSH_KILL_AFTER_TERM: Duration = Duration::from_secs(3);

/// How long the pump waits for its reader and its writer once a session is ending, which
/// outlasts the kick's SIGKILL, since something that `ssh` left behind may hold the pipes
/// open past it. So a deliberate end holds a raw terminal for at most this, a write blocked
/// on a daemon that has stopped answering included.
const WIND_DOWN_DEADLINE: Duration = Duration::from_secs(5);

/// How long the writer has to finish what is queued and close its half before the kick.
/// A healthy transport takes it at once, which is what lets the bridge's clean exit come
/// before the kick; only a write that is not moving outlasts this, and the kick is what
/// wakes that.
const WRITER_CLOSE_GRACE: Duration = Duration::from_millis(500);

/// How long a key goes with no frame read from the daemon after it before the bottom row
/// says the daemon is not answering (the owner's rulings of 2026-11-07 and, on E10r's
/// review 3, of 2026-11-08). An idle daemon writes an attached TUI about 185 frames a second
/// and a stopped one none, so this is a daemon that has stopped or is busy past it, on
/// either route and whatever the transport's buffers hold.
const NO_ANSWER_NOTICE: Duration = Duration::from_secs(2);

/// How many bytes wait for the transport before further input is dropped: one largest
/// frame, so a paste as large as a frame can be takes an empty queue (the owner's ruling of
/// 2026-11-07). A frame is taken while the queue is under this, so it holds at most twice
/// this.
const WRITE_QUEUE_CAP: usize = MAX_FRAME_BYTES;

/// A test's override of [`WRITE_QUEUE_CAP`], in bytes.
pub const PMACS_TEST_WRITE_QUEUE_CAP: &str = "PMACS_TEST_WRITE_QUEUE_CAP";

/// How much the writer gives the transport at a time, so that the queue's count, which the
/// cap reads, falls while a large frame goes out and not only once it has gone.
const WRITE_CHUNK: usize = 64 * 1024;

/// How long a reconnect over SSH waits for its `Hello` before it kills `ssh` and that
/// is one failed attempt on the schedule, a daemon that cannot be reached, which backs
/// off with no limit. 120 s by the owner's ruling of 2026-11-06, since `ssh` runs with no
/// `BatchMode` and may be asking for a passphrase: a shorter wait kills someone typing.
const RECONNECT_HELLO_DEADLINE: Duration = Duration::from_mins(2);

/// A test's override of [`RECONNECT_HELLO_DEADLINE`], in milliseconds.
pub const PMACS_TEST_RECONNECT_HELLO_MS: &str = "PMACS_TEST_RECONNECT_HELLO_MS";

/// How long `ssh` has to leave after a `SIGTERM` this client gave it during a handshake
/// before it has a `SIGKILL`. A stand-in that ignores one must not hold the route.
const SSH_TERM_GRACE: Duration = Duration::from_secs(1);

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
    /// The far side wrote text where a frame was expected, which is how a shell's
    /// complaint, `ssh`'s own and the bridge's errors arrive: the protocol rides `ssh`'s
    /// stderr ([`SSH_PROTOCOL_DEFAULT`]), so they were read as a frame's length, past the
    /// cap, and said `frame length 1650553704` (#477).
    FarSideSaid {
        /// Its exit code if it had one; `None` for a signal.
        code: Option<i32>,
        /// One line of what it said, with no control characters ([`far_side_line`]).
        words: String,
    },
    /// The daemon an attached session was on is gone: its socket refused
    /// the connect, or is not there.
    DaemonGone {
        /// The socket the session was attached to.
        socket: PathBuf,
        /// What the connect found.
        found: &'static str,
    },
    /// `C-c` in a reconnect's wait for `Hello`, which ends the route with nothing to say.
    ReconnectCancelled,
    /// A reconnect's `Hello` did not come within [`RECONNECT_HELLO_DEADLINE`].
    ReconnectHelloTimedOut {
        /// How long it waited.
        after: Duration,
    },
    /// [`HANDSHAKE_RETRY_CAP`] new daemons in a row each died young on the far side, so
    /// the remote route stopped trying ([`DyingDaemons`]).
    NewDaemonsKeepDying {
        /// What the client knows of where the last one's stderr is.
        stderr: FarSideStderr,
        /// How many in a row.
        attempts: u32,
    },
    /// [`HANDSHAKE_RETRY_CAP`] sessions in a row ended before their first
    /// frame, so the route stopped trying.
    SessionsDropped {
        /// How many in a row.
        attempts: u32,
        /// Whether the last of them ended with a frame cut short.
        cut_short: bool,
        /// Where a daemon that closed them keeps its stderr, for a remote route whose
        /// last session ended at a frame's boundary with `ssh` exiting 0: the bridge starts
        /// a daemon where it finds none, and one that dies this young has likely left its
        /// reason there. The local route starts no daemon, and a frame cut short is the
        /// daemon's bound, so neither names one.
        far_side: Option<FarSideStderr>,
    },
    /// [`HANDSHAKE_RETRY_CAP`] sessions in a row ended before their first frame with `ssh`
    /// exiting other than 0, so no daemon can be said to have done anything and no file
    /// is named.
    SshEndedBeforeFirstFrame {
        /// The host, as the route was given it.
        host: String,
        /// How many in a row.
        attempts: u32,
        /// How the last `ssh` ended.
        exit: SshExit,
        /// The far side's last words, if it left any.
        said: Option<String>,
    },
}

/// How a session's `ssh` ended. The bridge exits 0 when its daemon closes the socket, so
/// anything else says the connection failed and says nothing of a daemon.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SshExit {
    /// It exited 0, or its exit could not be read.
    #[default]
    Clean,
    /// It exited with this status.
    Code(i32),
    /// A signal ended it.
    Signal,
}

impl SshExit {
    fn of(status: Option<std::process::ExitStatus>) -> Self {
        match status {
            Some(status) if !status.success() => status.code().map_or(Self::Signal, Self::Code),
            _ => Self::Clean,
        }
    }
}

impl std::fmt::Display for SshExit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Clean => write!(f, "status 0"),
            Self::Code(code) => write!(f, "status {code}"),
            Self::Signal => write!(f, "a signal"),
        }
    }
}

/// The host a remote route names, with the user where the route was given one, since
/// `ssh` to the bare host may read another user's directory.
fn route_host(target: &AttachTarget) -> String {
    match target {
        AttachTarget::Ssh {
            host,
            user: Some(user),
            ..
        } => format!("{user}@{host}"),
        AttachTarget::Ssh { host, .. } => host.clone(),
        _ => "the far side".to_owned(),
    }
}

/// Where a daemon the remote route's bridge started keeps its stderr, as the client can
/// say it: the host, and the socket's name, or that it is the default, under the far
/// side's runtime directory, whose real path no message says and no wire change is taken
/// to learn. A word a user will paste is not a variable their own shell would expand, so
/// the sentence says whose it is.
#[derive(Debug)]
pub struct FarSideStderr {
    /// The host, with the user where the route was given one.
    host: String,
    /// The file, under the far side's runtime directory.
    path: String,
}

impl FarSideStderr {
    fn of(target: &AttachTarget) -> Self {
        let instance = match target {
            AttachTarget::Ssh { instance_name, .. } => instance_name.as_deref(),
            _ => None,
        };
        let socket = crate::socket_path::resolve_socket_path_with_runtime(
            instance,
            std::path::Path::new(""),
        );
        let path = pmacs_protocol::socket_file::daemon_stderr_path(&socket);
        Self {
            host: route_host(target),
            path: path.display().to_string(),
        }
    }
}

impl std::fmt::Display for FarSideStderr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.path.starts_with('/') {
            // A path given for the socket is the far side's own.
            return write!(f, "{}", self.path);
        }
        write!(
            f,
            "{} under that host's $XDG_RUNTIME_DIR, or /tmp/pmacs-<uid> where that is unset",
            self.path
        )
    }
}

impl std::fmt::Display for AttachError {
    // One arm a variant, each a sentence a user reads, with nothing shared to pull out.
    #[allow(clippy::too_many_lines)]
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
                write_ssh_exit(f, *code, &tail_hint)
            }
            Self::FarSideSaid { code, words } => {
                write!(f, "the far side said: {words}; ")?;
                write_ssh_exit(f, *code, "")
            }
            Self::DaemonGone { socket, found } => write!(
                f,
                "the daemon at {} is gone ({found}), so this session has ended; start it again \
                 and attach",
                socket.display()
            ),
            Self::ReconnectCancelled => write!(f, "the reconnect was cancelled"),
            Self::ReconnectHelloTimedOut { after } => write!(
                f,
                "the far side did not say hello within {} s",
                after.as_secs()
            ),
            Self::NewDaemonsKeepDying { stderr, attempts } => write!(
                f,
                "a new daemon on {host} died {attempts} times in a row, each session lasting \
                 under 30 s, so this client has stopped trying; the last one's stderr is in \
                 {stderr}",
                host = stderr.host,
            ),
            Self::SessionsDropped {
                attempts,
                cut_short: true,
                ..
            } => write!(
                f,
                "{FRAME_CUT_SHORT_SENTENCE}; stopped after {attempts} sessions in a row ended \
                 this way"
            ),
            Self::SessionsDropped {
                attempts,
                cut_short: false,
                far_side: None,
            } => write!(
                f,
                "the daemon closed {attempts} sessions in a row before their first frame, so \
                 this one has stopped trying"
            ),
            Self::SessionsDropped {
                attempts,
                cut_short: false,
                far_side: Some(stderr),
            } => write!(
                f,
                "the daemon on {host} closed {attempts} sessions in a row before their first \
                 frame, so this client has stopped trying; if pmacs started it, its stderr is in \
                 {stderr}",
                host = stderr.host,
            ),
            Self::SshEndedBeforeFirstFrame {
                host,
                attempts,
                exit,
                said,
            } => {
                write!(
                    f,
                    "ssh to {host} ended {attempts} times in a row before a first frame ({exit}), \
                     so this client has stopped trying"
                )?;
                said.as_ref()
                    .map_or(Ok(()), |words| write!(f, "; it last said: {words}"))
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
            Self::VersionMismatch { .. }
            | Self::Rejected(_)
            | Self::SshChildExited { .. }
            | Self::FarSideSaid { .. }
            | Self::DaemonGone { .. }
            | Self::ReconnectCancelled
            | Self::ReconnectHelloTimedOut { .. }
            | Self::NewDaemonsKeepDying { .. }
            | Self::SessionsDropped { .. }
            | Self::SshEndedBeforeFirstFrame { .. } => None,
        }
    }
}

impl AttachError {
    /// What the far side said, for a reconnect's countdown to say why.
    fn far_side_words(&self) -> Option<String> {
        let words = match self {
            Self::FarSideSaid { words, .. } => words.clone(),
            Self::SshChildExited { stderr_tail, .. } => format_stderr_tail_for_display(stderr_tail),
            _ => String::new(),
        };
        (!words.is_empty()).then_some(words)
    }
}

/// What an `ssh` that ended with `code` means, with `hint` after it.
fn write_ssh_exit(
    f: &mut std::fmt::Formatter<'_>,
    code: Option<i32>,
    hint: &str,
) -> std::fmt::Result {
    match code {
        Some(127) => write!(
            f,
            "SSH session exited 127 (command not found on remote). \
             This usually means `pmacs` is not on the remote PATH for \
             non-interactive SSH. Try: `ssh <host> 'which pmacs'` to \
             verify{hint}"
        ),
        Some(c) => write!(f, "SSH session exited {c}{hint}"),
        None => write!(f, "SSH session terminated by signal (no exit code){hint}"),
    }
}

/// How much of a far side's line reaches a terminal.
const FAR_SIDE_LINE_CHARS: usize = 200;

/// Whether `c` is a bidirectional control: an override or an isolate a far side left open
/// would reorder what a terminal that does bidi draws after it, pmacs's own words included,
/// so these go with the controls (E10r's review 3, ruled 2026-11-08).
fn is_bidi_control(c: char) -> bool {
    matches!(
        c,
        '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

/// What of a far side's text never reaches a terminal: a control, C0 or C1, or a
/// bidirectional one.
fn unshown(c: char) -> bool {
    c.is_control() || is_bidi_control(c)
}

/// One line of what a far side said, fit to reach a terminal: its last line with text and
/// no control character, since what follows a far side's text may be a frame's bytes, or
/// failing that its last line with them stripped, and the end of it if it is long.
/// A far side's bytes are not to reach a terminal as they came: an escape sequence in them
/// could rewrite the screen or the window's title, and a bidirectional override reorder it.
fn far_side_line(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let has_text = |line: &&str| !line.trim().is_empty();
    let clean = |line: &&str| {
        has_text(line)
            && !line
                .chars()
                .any(|c| unshown(c) || c == char::REPLACEMENT_CHARACTER)
    };
    let line = text
        .lines()
        .rfind(clean)
        .or_else(|| text.lines().rfind(has_text))
        .unwrap_or("");
    let kept: Vec<char> = line.trim().chars().filter(|c| !unshown(*c)).collect();
    let from = kept.len().saturating_sub(FAR_SIDE_LINE_CHARS);
    let tail: String = kept[from..].iter().collect();
    if from == 0 {
        tail
    } else {
        format!("...{tail}")
    }
}

/// Trim the captured stderr tail to a single short line for
/// inclusion in the [`AttachError::SshChildExited`] message. The
/// full bytes are available via the `stderr_tail` field; this is
/// just a one-liner preview.
fn format_stderr_tail_for_display(tail: &str) -> String {
    far_side_line(tail.as_bytes())
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
    /// Writer half. Moved into the writer thread ([`FrameWriter`]), so that a write
    /// that blocks holds nothing that reads the terminal.
    pub writer: Box<dyn Write + Send>,
    /// Force the reader and writer threads to exit promptly, by any
    /// means necessary.
    ///
    /// Called by the main thread after the message loop has decided
    /// to terminate, immediately before joining them. The reader
    /// thread is presumed to be blocked on a read, and the writer may
    /// be blocked on a write; this wakes them.
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

    /// [`Self::present_messages`] with `text` standing on the bottom row, over what
    /// the frames put there. A test's stub has no screen, so by default it only presents.
    fn present_under_notice(
        &mut self,
        msgs: &[InstanceMessage],
        _text: &str,
    ) -> std::io::Result<()> {
        self.present_messages(msgs)
    }

    /// Take a notice off the bottom row, leaving what the frames put there.
    fn clear_notice(&mut self) -> std::io::Result<()> {
        Ok(())
    }

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
    fn present_under_notice(
        &mut self,
        msgs: &[InstanceMessage],
        text: &str,
    ) -> std::io::Result<()> {
        Frontend::present_under_notice(self, msgs, text)
    }
    fn clear_notice(&mut self) -> std::io::Result<()> {
        Frontend::clear_notice(self)
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

/// How long a new session of the local route waits for its `Hello`. A
/// daemon that takes the connect and says nothing costs an attempt, not the
/// terminal, whose keys nothing reads during a handshake.
const LOCAL_RECONNECT_HELLO_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a notice stands whatever is typed under it: a user typing when a link
/// drops is the likeliest to be at the keyboard, and at 5 keys a second it stood for
/// 128 ms when any key took it off (review 1, ruled 2026-11-06). Keys reach the daemon
/// under it. If any arrived during this it comes off by itself at the end of it, so that
/// a prompt the sentence sent them to shows; if none did it stands until one does,
/// whatever frames take its row, so that it is not cleared on a terminal nobody is at
/// (review 2, ruled 2026-11-07).
const NOTICE_FLOOR: Duration = Duration::from_secs(5);

/// On the bottom row of a session that replaced a lost one, for [`NOTICE_FLOOR`] and
/// until the next key: the view is fresh, so the user is told why the buffer changed.
const NOTICE_FRESH_VIEW: &str = "[pmacs reconnected on a new view: the daemon closed the last one]";

/// [`NOTICE_FRESH_VIEW`] when the last one ended with a frame cut short: the
/// daemon's bound, in what a terminal's bottom row has room for.
const NOTICE_FRESH_VIEW_CUT_SHORT: &str =
    "[pmacs reconnected on a new view: a frame did not arrive within 2 s]";

/// [`NOTICE_FRESH_VIEW`] when the SSH route's `ssh` failed, which says nothing of
/// the daemon: the bridge exits 0 when the daemon closes its socket.
const NOTICE_FRESH_VIEW_LINK: &str = "[pmacs reconnected on a new view: the connection was lost]";

/// On the bottom row, over anything else, once a key has had no frame from the daemon
/// after it for [`NO_ANSWER_NOTICE`], and off at the next frame read. The owner's words,
/// ruled 2026-11-07, and their trigger, ruled 2026-11-08: keys typed meanwhile are queued,
/// in order, and `F12` is always read.
const NOTICE_NOT_ANSWERING: Notice = Notice {
    wide: "[pmacs: the daemon is not answering; F12 detaches]",
    narrow: Some("[not answering; F12 detaches]"),
};

/// [`NOTICE_NOT_ANSWERING`] once input has been dropped at [`WRITE_QUEUE_CAP`]. The owner's
/// words, ruled 2026-11-07.
const NOTICE_KEYS_DROPPED: Notice = Notice {
    wide: "[pmacs: the daemon is not answering; keys dropped; F12 detaches]",
    narrow: Some("[keys dropped; F12 detaches]"),
};

/// On the bottom row of a session on a daemon `Hello` says for certain is not the
/// last session's. The owner's words, ruled 2026-11-06: a modified file buffer's copy
/// is what the next daemon offers, and `autosave.interval-ms` is how old it may be.
const NOTICE_NEW_DAEMON: Notice = Notice {
    wide: "[pmacs: new daemon; work not autosaved is gone; M-x recover-file]",
    narrow: Some("[new daemon; work not autosaved is gone]"),
};

/// On the bottom row of a session on a daemon `Hello` cannot tell from the last
/// session's. The owner's words, ruled 2026-11-06: a daemon read this way may hold
/// everything, so this tells the user where to look and calls nothing gone.
const NOTICE_MAYBE_NEW_DAEMON: Notice = Notice {
    wide: "[pmacs reconnected: this may be a new daemon; C-x b shows what it holds]",
    narrow: Some("[this may be a new daemon]"),
};

/// What goes on a new session's bottom row, and what goes there on a pane too narrow
/// for it, so a sentence is not cut before the clause that matters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Notice {
    wide: &'static str,
    narrow: Option<&'static str>,
}

impl Notice {
    /// A notice with one form, which a narrow pane cuts.
    const fn plain(text: &'static str) -> Self {
        Self {
            wide: text,
            narrow: None,
        }
    }

    /// The form that fits a pane `cols` wide.
    fn fitting(self, cols: u32) -> &'static str {
        let fits = u32::try_from(self.wide.chars().count()).is_ok_and(|width| width <= cols);
        match self.narrow {
            Some(narrow) if !fits => narrow,
            _ => self.wide,
        }
    }
}

/// What a route keeps between sessions, to say why a new one is a new view.
#[derive(Debug, Default)]
struct SessionMemory {
    /// What the last session's `Hello` said of its daemon.
    seen: Option<DaemonSeen>,
    /// What the last session's own `Hello` read.
    reading: Succession,
    /// The strongest thing a `Hello` has said of a new daemon since a session on one
    /// reached the first frame that puts it on screen. It outlasts a session lost
    /// before that, whose successor's `Hello` is the new daemon's own.
    succession: Succession,
    /// Why the last session was lost.
    why: Option<&'static str>,
}

impl SessionMemory {
    /// A session's `Hello` said this.
    fn heard(&mut self, hello: &Hello) {
        self.heard_at(
            hello.assigned_frontend_id,
            hello.instance_identity.uptime_secs,
            Instant::now(),
        );
    }

    fn heard_at(&mut self, id: FrontendId, uptime_secs: u64, at: Instant) {
        let seen = DaemonSeen::new(id, uptime_secs, at);
        self.reading = self
            .seen
            .map_or(Succession::Same, |last| last.succeeded_by(&seen));
        self.succession = self.succession.max(self.reading);
        self.seen = Some(seen);
    }

    /// A session was lost, for the reason `why` says of it. `told` is whether what a
    /// `Hello` said has been on screen for its floor: a first frame has put it up and it
    /// has stood. Until then it is kept for the next session, so a link lost inside the
    /// floor does not take the one sentence that says work is gone.
    fn lost(&mut self, told: bool, why: &'static str) {
        if told {
            self.succession = Succession::Same;
        }
        self.why = Some(why);
    }

    /// How long since the last session's `Hello`: how long it lived, when it is lost.
    fn session_lived(&self) -> Duration {
        self.seen.map_or(Duration::ZERO, |seen| seen.age())
    }

    /// What the next session puts on its bottom row.
    fn notice(&self) -> Option<Notice> {
        match self.succession {
            Succession::Certain => Some(NOTICE_NEW_DAEMON),
            Succession::Maybe => Some(NOTICE_MAYBE_NEW_DAEMON),
            Succession::Same => self.why.map(Notice::plain),
        }
    }
}

/// Connect to the daemon at `socket_path` and run the attach client.
///
/// Returns when the session ends on purpose (F12, or the daemon's `Goodbye`),
/// or when it cannot go on: the daemon is gone, or sessions keep ending before
/// their first frame. A socket that ends with no `Goodbye` while the daemon is
/// there is attached again, on a fresh view.
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
    let hello = read_local_hello(&mut stream)?;
    print_attach_info(&hello);
    let mut memory = SessionMemory::default();
    memory.heard(&hello);

    let (cols, rows) = crossterm::terminal::size().map_err(AttachError::Terminal)?;
    let initial_size = CellSize::new(u32::from(rows), u32::from(cols));
    write_attach_request(&mut stream, &hello, initial_size)?;

    // Take over the terminal. Frontend's Drop tears it down.
    let mut frontend = Frontend::new().map_err(AttachError::Terminal)?;

    let io = build_local_socket_io(stream)?;
    let result = run_local_sessions(
        &socket_path,
        io,
        hello.assigned_frontend_id,
        &mut frontend,
        &mut memory,
    );

    // Frontend drops here; raw mode + alternate screen + mouse capture
    // all torn down before we return, so what the caller prints of the
    // result lands on the terminal the user is left with.
    drop(frontend);

    said_goodbye(result?)
}

/// What a daemon's `Goodbye` said, once the alternate screen is left and it can be read,
/// where the pump could only have written it to the screen it was leaving. A refusal is
/// an error, which ends the process saying so. A frontend's own quit (`Detached`) and
/// its own `F12` (`None`) say nothing to whoever did them.
fn said_goodbye(reason: Option<GoodbyeReason>) -> Result<(), AttachError> {
    match reason {
        None | Some(GoodbyeReason::Detached) => Ok(()),
        Some(GoodbyeReason::ShuttingDown) => {
            eprintln!(
                "pmacs: instance disconnected: {:?}",
                GoodbyeReason::ShuttingDown
            );
            Ok(())
        }
        Some(refused) => Err(AttachError::Rejected(refused)),
    }
}

/// Read a daemon's `Hello` and check its version.
fn read_local_hello(stream: &mut UnixStream) -> Result<Hello, AttachError> {
    let hello: Hello = read_message(stream)?;
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
    Ok(hello)
}

/// Answer `hello` with this frontend's `AttachRequest`.
fn write_attach_request(
    stream: &mut UnixStream,
    hello: &Hello,
    initial_size: CellSize,
) -> Result<(), AttachError> {
    // T M10.5: match the server's protocol version so a v1.0 frontend
    // connecting to a v0.1 daemon advertises protocol_version=1 in
    // its AttachRequest (the v0.1 daemon's strict-equality check will
    // accept). The frontend's runtime behavior on the wire is the
    // intersection of features both sides support.
    //
    // Bottom-panel Stage 2B-3: that echo is now a *floor*, not the whole
    // rule. `Hello` is server-first, so the daemon must advertise a
    // baseline every shipped frontend accepts; the session's real version
    // is settled here, by this frontend counter-offering its own
    // `PROTOCOL_VERSION` when the baseline is the current one. Anything
    // older is still echoed verbatim.
    let req = AttachRequest {
        protocol_version: crate::protocol::requested_protocol_version(hello.protocol_version),
        frontend_capabilities: build_capabilities(),
        initial_size,
    };
    write_message(stream, &req)?;
    Ok(())
}

/// The local route's sessions, one after another for as long as each is lost
/// and the daemon is there.
///
/// The schedule starts again after a session that reached its first frame, and
/// so does the count of those that did not, which [`HANDSHAKE_RETRY_CAP`]
/// bounds so a daemon that drops every new session in its bootstrap holds
/// nobody up for long and this route cannot spin. Unlike the SSH route's, a
/// schedule here restarts: a loss is an event of its own on a local socket, not
/// a network that may stay down, and it starts from a daemon known to be there.
fn run_local_sessions(
    socket_path: &std::path::Path,
    first: AttachIo,
    first_id: FrontendId,
    frontend: &mut Frontend,
    memory: &mut SessionMemory,
) -> Result<Option<GoodbyeReason>, AttachError> {
    let mut schedule = BackoffSchedule::new();
    let mut unbootstrapped: u32 = 0;
    let mut session = (first, first_id);
    let mut cut_short;
    loop {
        let (io, id) = session;
        let lost = match run_attach_pair(io, frontend, id, memory.notice())? {
            PumpEnd::Over(reason) => return Ok(reason),
            PumpEnd::Lost(lost) => lost,
        };
        memory.lost(
            lost.bootstrapped && lost.notice_stood,
            if lost.cut_short {
                NOTICE_FRESH_VIEW_CUT_SHORT
            } else {
                NOTICE_FRESH_VIEW
            },
        );
        if lost.bootstrapped {
            schedule.reset();
            unbootstrapped = 0;
        } else {
            unbootstrapped = unbootstrapped.saturating_add(1);
        }
        cut_short = lost.cut_short;
        // A loss is usually a stop, and a stop may have left the terminal cooked.
        let _ = frontend.retake_terminal();
        session = loop {
            if unbootstrapped >= HANDSHAKE_RETRY_CAP {
                return Err(AttachError::SessionsDropped {
                    attempts: unbootstrapped,
                    cut_short,
                    far_side: None,
                });
            }
            let delay = schedule.next_delay();
            match sleep_with_countdown(Some(&mut *frontend), delay, None) {
                SleepOutcome::Cancelled => return Ok(None),
                SleepOutcome::Elapsed => {}
            }
            let _ = frontend.draw_status_overlay(banner_session_reconnect());
            match connect_again(socket_path, frontend.size(), memory) {
                Ok(next) => break next,
                Err(
                    error @ (AttachError::DaemonGone { .. } | AttachError::VersionMismatch { .. }),
                ) => return Err(error),
                Err(_) => unbootstrapped = unbootstrapped.saturating_add(1),
            }
        };
    }
}

/// One new session on the local route's socket: its connect and handshake.
fn connect_again(
    socket_path: &std::path::Path,
    size: CellSize,
    memory: &mut SessionMemory,
) -> Result<(AttachIo, FrontendId), AttachError> {
    let mut stream = UnixStream::connect(socket_path).map_err(|error| {
        use pmacs_protocol::{ConnectFailure, classify_connect_failure};
        let found = match classify_connect_failure(socket_path, &error) {
            ConnectFailure::Missing => "no socket is there",
            ConnectFailure::Stale => "its socket refused the connection",
            ConnectFailure::NotASocket => "what is there is not a socket",
            ConnectFailure::Other | ConnectFailure::Inspect(_) => return AttachError::Io(error),
        };
        AttachError::DaemonGone {
            socket: socket_path.to_path_buf(),
            found,
        }
    })?;
    stream.set_read_timeout(Some(LOCAL_RECONNECT_HELLO_TIMEOUT))?;
    let hello = read_local_hello(&mut stream)?;
    memory.heard(&hello);
    stream.set_read_timeout(None)?;
    write_attach_request(&mut stream, &hello, size)?;
    Ok((build_local_socket_io(stream)?, hello.assigned_frontend_id))
}

/// Build an [`AttachIo`] for a connected `UnixStream`.
///
/// The kick sets a shared flag and clones a third handle for
/// `shutdown(Both)`. Cloning may
/// fail (rare — the kernel is out of file descriptors), in which
/// case the caller propagates the error before raw mode engages.
fn build_local_socket_io(stream: UnixStream) -> Result<AttachIo, std::io::Error> {
    let reader = stream.try_clone()?;
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

/// A reader that waits in `poll` with a kick flag.
///
/// The socket stays blocking and is read only once `poll` says it holds
/// something, so data is taken as it arrives. Until E10r this was a
/// non-blocking read with a 10 ms sleep between tries, which had two
/// costs: a frame larger than the socket's buffer arrived a bufferful per
/// sleep, about 30 MB/s, so a healthy TUI held the daemon's dispatcher for
/// 630 ms on a 16.5 MB frame where the GPU's reader took 5 ms; and
/// `O_NONBLOCK` is shared across clones, so the writer was non-blocking
/// too, and a paste larger than the socket's buffer failed with `EAGAIN`
/// and ended the session.
///
/// # Wake semantics
///
/// This reader has two cooperating wake paths, only one of which is
/// load-bearing:
///
/// 1. **Atomic flag (correctness):** `poll` waits at most 10 ms. After
///    the kick sets `kicked`, the first wait that times out observes it
///    and returns `Ok(0)`. Worst-case wake latency is one poll cycle
///    (~10ms). This path is platform-independent and is the mechanism
///    the caller relies on for correctness.
///
/// 2. **`shutdown(Both)` on a sibling clone (best-effort speedup):**
///    if the platform honors cross-clone shutdown wakes, `poll` returns
///    at once and the read returns `Ok(0)`. This path is **not**
///    load-bearing — Unix socket cross-clone shutdown semantics are not
///    portably guaranteed, and any wake it provides is a bonus on top
///    of path 1.
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
            let mut fds = [PollFd::new(self.stream.as_fd(), PollFlags::POLLIN)];
            match poll(&mut fds, PollTimeout::from(10u16)) {
                Ok(0) | Err(Errno::EINTR) => {
                    if self.kicked.load(Ordering::SeqCst) {
                        return Ok(0);
                    }
                }
                Ok(_) => return self.stream.read(buf),
                Err(errno) => return Err(errno.into()),
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
    capabilities_for(crate::color_depth::ColorDepth::detect_env())
}

/// [`build_capabilities`] for a terminal detected to show `depth`
/// (E10g.5): `true_color` says whether it shows truecolor, which is all
/// the daemon learns of it; a client at 16 reduces what it is sent.
fn capabilities_for(depth: crate::color_depth::ColorDepth) -> FrontendCapabilities {
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
        true_color: depth == crate::color_depth::ColorDepth::TrueColor,
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

/// What the pump and its writer share.
#[derive(Debug)]
struct WriteQueue {
    frames: VecDeque<Vec<u8>>,
    /// Bytes not yet given to the transport, those of the frame in hand included.
    bytes: usize,
    /// The writer has a frame in hand.
    writing: bool,
    /// Input has been dropped at the cap since the queue last drained.
    dropped: bool,
    /// What a write failed with: the writer has stopped, and whatever was queued goes
    /// with the session.
    failure: Option<io::Error>,
    /// How far the writer's end has come.
    end: WriterEnd,
}

/// How far a [`FrameWriter`]'s end has come.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WriterEnd {
    /// More may come.
    Open,
    /// Nothing more comes: the writer ends when the queue does.
    Closing,
    /// It has ended and closed its half of the transport.
    Closed,
}

/// Frames to the daemon, encoded on the pump and written in order by one thread on either
/// route, so that the thread reading the terminal never waits on the transport: `F12` is
/// read whatever the socket is doing (E10r's review 2, ruled 2026-11-07). Until then the
/// pump wrote them itself, and a user typing at a daemon that had stopped answering could
/// not leave: 200 keys filled a local socket and 4,100 a remote route's, and then nothing
/// read `F12` or `C-c`.
///
/// A bounded write was the other way, and it would have ended the session whenever a long
/// command outlasted the bound under typing, with one form for a socket and another for
/// `ssh`'s pipe.
struct FrameWriter {
    shared: Arc<(Mutex<WriteQueue>, Condvar)>,
    thread: thread::JoinHandle<()>,
    cap: usize,
}

/// What the pump reads of the writer on each turn.
struct WriterState {
    /// Something is queued or in hand.
    pending: bool,
    /// Input has been dropped since the queue last drained.
    dropped: bool,
}

impl FrameWriter {
    fn spawn(writer: Box<dyn Write + Send>) -> Self {
        let shared = Arc::new((
            Mutex::new(WriteQueue {
                frames: VecDeque::new(),
                bytes: 0,
                writing: false,
                dropped: false,
                failure: None,
                end: WriterEnd::Open,
            }),
            Condvar::new(),
        ));
        let for_thread = Arc::clone(&shared);
        let thread = thread::spawn(move || run_writer(writer, &for_thread));
        let cap = std::env::var(PMACS_TEST_WRITE_QUEUE_CAP)
            .ok()
            .and_then(|bytes| bytes.parse().ok())
            .unwrap_or(WRITE_QUEUE_CAP);
        Self {
            shared,
            thread,
            cap,
        }
    }

    fn queue(&self) -> std::sync::MutexGuard<'_, WriteQueue> {
        self.shared.0.lock().expect("the writer's queue")
    }

    /// Queue one encoded frame, or several, for the writer. Nothing happens for none,
    /// which is what a key's release encodes to.
    fn push(&self, frame: Vec<u8>) {
        if frame.is_empty() {
            return;
        }
        let mut queue = self.queue();
        queue.bytes = queue.bytes.saturating_add(frame.len());
        queue.frames.push_back(frame);
        self.shared.1.notify_all();
    }

    /// Encode `event` here on the pump, so a frame too large says so where the session
    /// decides what that means, and queue it.
    fn send(&self, event: &FrontendEvent) -> Result<(), AttachError> {
        let mut frame = Vec::new();
        write_message(&mut frame, event)?;
        self.push(frame);
        Ok(())
    }

    /// Whether further input is dropped.
    fn full(&self) -> bool {
        self.queue().bytes >= self.cap
    }

    /// Some input was dropped at the cap.
    fn input_dropped(&self) {
        self.queue().dropped = true;
    }

    fn take_failure(&self) -> Option<io::Error> {
        self.queue().failure.take()
    }

    fn state(&self) -> WriterState {
        let queue = self.queue();
        WriterState {
            pending: queue.writing || !queue.frames.is_empty(),
            dropped: queue.dropped,
        }
    }

    /// Nothing more comes, so the writer finishes what is queued and closes its half of
    /// the transport, which on the SSH route is what starts the bridge's clean exit.
    /// Returns when it has, or after `grace` with a write that is not moving, which the
    /// kick wakes.
    fn close_within(&self, grace: Duration) {
        let deadline = Instant::now() + grace;
        let mut queue = self.queue();
        queue.end = WriterEnd::Closing;
        self.shared.1.notify_all();
        while queue.end != WriterEnd::Closed {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return;
            }
            queue = self
                .shared
                .1
                .wait_timeout(queue, left)
                .expect("the writer's queue")
                .0;
        }
    }
}

/// The writer's thread: each frame in the order it came, until the queue is closed and
/// empty or a write fails, and its half of the transport closes as it ends.
fn run_writer(mut writer: Box<dyn Write + Send>, shared: &(Mutex<WriteQueue>, Condvar)) {
    let (queue, wake) = shared;
    'frames: loop {
        let frame = {
            let mut held = queue.lock().expect("the writer's queue");
            loop {
                if let Some(frame) = held.frames.pop_front() {
                    held.writing = true;
                    break frame;
                }
                held.writing = false;
                held.dropped = false;
                if held.end == WriterEnd::Closing {
                    break 'frames;
                }
                held = wake.wait(held).expect("the writer's queue");
            }
        };
        for chunk in frame.chunks(WRITE_CHUNK) {
            let written = writer.write_all(chunk).and_then(|()| writer.flush());
            let mut held = queue.lock().expect("the writer's queue");
            if let Err(error) = written {
                held.failure = Some(error);
                held.writing = false;
                break 'frames;
            }
            held.bytes = held.bytes.saturating_sub(chunk.len());
        }
    }
    drop(writer);
    queue.lock().expect("the writer's queue").end = WriterEnd::Closed;
    wake.notify_all();
}

/// Whether the daemon answers what this client sends it, read by its frames (E10r's review
/// 3, ruled 2026-11-08): an idle daemon writes an attached TUI about 185 frames a second and a
/// stopped one none, so a key with no frame read after it for [`NO_ANSWER_NOTICE`] is a
/// daemon not answering, on either route, where the transport's being full of unsent keys,
/// the trigger before it, came only after 337 keys locally and about 110,000 behind `ssh`'s
/// window. A frame already on its way when a key goes answers it all the same, so a single
/// key that starts a long hold may say nothing until the next key, and a frame reads as an
/// answer whatever it holds: if a daemon ever stops writing frames while idle (#457), each
/// input must still have one after it, or this reads a quiet daemon as one not answering.
#[derive(Debug)]
struct Answers {
    /// When the reader last read a frame, set on its thread.
    last_frame: Arc<Mutex<Instant>>,
    /// When the first input went that no frame has been read after.
    unanswered_since: Option<Instant>,
}

impl Answers {
    fn new() -> Self {
        Self {
            last_frame: Arc::new(Mutex::new(Instant::now())),
            unanswered_since: None,
        }
    }

    /// Input is about to go to the daemon: a key, or a paste.
    fn input_sent(&mut self) {
        self.unanswered_since.get_or_insert_with(Instant::now);
    }

    /// How long the oldest input with no frame read after it has waited, if one has.
    fn unanswered_for(&mut self) -> Option<Duration> {
        let since = self.unanswered_since?;
        if *self.last_frame.lock().expect("the last frame's time") > since {
            self.unanswered_since = None;
            return None;
        }
        Some(since.elapsed())
    }
}

/// What stands over the frames on a terminal's bottom row: a new session's notice, for
/// [`NOTICE_FLOOR`] and until the next key, and over it, while it lasts, that the daemon is
/// not answering, which is the sentence that says how to leave. When that one goes the
/// other comes back, if it has not stood its floor or no key has come.
#[derive(Debug, Default)]
struct BottomRow {
    /// A new session's notice.
    standing: Option<StandingNotice>,
    /// That the daemon is not answering, and whether keys are being dropped.
    stalled: Option<Notice>,
    /// What is on the row now.
    painted: Option<&'static str>,
}

/// A new session's notice on the bottom row.
#[derive(Clone, Copy, Debug)]
struct StandingNotice {
    notice: Notice,
    since: Instant,
    /// A key arrived during its floor.
    keyed: bool,
}

impl StandingNotice {
    fn new(notice: Notice) -> Self {
        Self {
            notice,
            since: Instant::now(),
            keyed: false,
        }
    }

    fn stood(&self) -> bool {
        self.since.elapsed() >= NOTICE_FLOOR
    }
}

impl BottomRow {
    fn wanted(&self, cols: u32) -> Option<&'static str> {
        self.stalled
            .or(self.standing.map(|standing| standing.notice))
            .map(|notice| notice.fitting(cols))
    }

    /// A key arrived: the notice comes off if it has stood its floor, and if not, it now
    /// comes off by itself when it has.
    fn key_arrived(&mut self) {
        if let Some(standing) = self.standing.as_mut() {
            standing.keyed = true;
        }
        self.time_passed();
    }

    /// Take the notice off if a key arrived during its floor and the floor is over.
    fn time_passed(&mut self) {
        if self.standing.is_some_and(|s| s.keyed && s.stood()) {
            self.standing = None;
        }
    }

    /// Whether the notice, if there was one, has stood its floor.
    fn notice_stood(&self) -> bool {
        self.standing.is_none_or(|standing| standing.stood())
    }

    /// Whether the daemon is answering: input dropped at the cap says so until the queue
    /// has drained, and otherwise a key [`NO_ANSWER_NOTICE`] old with no frame read after it
    /// says the daemon is not answering, until the next frame.
    fn daemon_says(&mut self, writer: &WriterState, unanswered_for: Option<Duration>) {
        self.stalled = if writer.pending && writer.dropped {
            Some(NOTICE_KEYS_DROPPED)
        } else if unanswered_for.is_some_and(|waited| waited >= NO_ANSWER_NOTICE) {
            Some(NOTICE_NOT_ANSWERING)
        } else {
            None
        };
    }

    /// Make the row what it should be, if it is not.
    fn settle(&mut self, frontend: &mut dyn AttachPumpFrontend) -> io::Result<()> {
        let wanted = self.wanted(frontend.size().cols);
        if wanted == self.painted {
            return Ok(());
        }
        match wanted {
            Some(text) => frontend.present_under_notice(&[], text)?,
            None => frontend.clear_notice()?,
        }
        self.painted = wanted;
        Ok(())
    }
}

/// How an attach pump ended, when it did not end on an error of its own.
#[derive(Debug)]
pub(crate) enum PumpEnd {
    /// The session ended on purpose: the daemon's `Goodbye`, with its reason, or F12.
    Over(Option<GoodbyeReason>),
    /// The socket ended with no `Goodbye`.
    Lost(LostSession),
}

/// What a lost session reached, for the loop deciding what comes next.
#[derive(Debug)]
pub(crate) struct LostSession {
    /// Whether its first frame arrived. A grid session's bootstrap is its
    /// snapshots and nothing marks its end, so the first `CellDelta`, which
    /// the first tick after the bootstrap writes, is what says so.
    pub bootstrapped: bool,
    /// The write that found the socket gone, when one did before the reader
    /// saw its end: a key typed while the frontend was stopped, say.
    pub write_error: Option<AttachError>,
    /// Whether the reader's last read ended partway through a frame.
    pub cut_short: bool,
    /// Whether the notice it had, if it had one, stood its floor.
    pub notice_stood: bool,
    /// What the far side said where a frame was expected, if it did.
    pub far_side_said: Option<String>,
}

/// Bidirectional pump: forward local input to the writer, apply
/// instance messages to the terminal.
///
/// Threading: a background thread reads instance messages from the
/// reader half and forwards them on a channel, and another writes the
/// frames this one encodes ([`FrameWriter`]). The main thread polls the
/// terminal for input, drains the channel, and renders, and waits on
/// neither transport, so `F12` is always read.
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
/// `notice`, if any, goes on the bottom row over the session's first frame. It stands
/// [`NOTICE_FLOOR`] whatever is typed, painted again over a frame that takes that row.
/// If a key arrived during that it comes off by itself at the end of it, and if none did
/// it stands until one does, what the frames left there coming back either way.
///
/// On any exit path — clean Goodbye, reader EOF, F12 detach, IO
/// error, or terminal error — `io.kick` fires and the reader and writer
/// threads are joined, within [`WIND_DOWN_DEADLINE`], before this function
/// returns. The closure-and-call wind-down pattern guarantees this
/// regardless of which `return` the loop takes.
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
    notice: Option<Notice>,
) -> Result<PumpEnd, AttachError> {
    let mut notice = notice;
    let mut bottom = BottomRow::default();
    let mut bootstrapped = false;
    let AttachIo {
        reader,
        writer,
        kick,
    } = io;

    let (tx, rx) = mpsc::channel::<InstanceMessage>();
    let far_side_text = Arc::new(Mutex::new(Vec::new()));
    let text_for_reader = Arc::clone(&far_side_text);
    let mut answers = Answers::new();
    let frame_for_reader = Arc::clone(&answers.last_frame);
    let reader_handle =
        thread::spawn(move || run_reader(reader, tx, &text_for_reader, &frame_for_reader));
    let writer = FrameWriter::spawn(writer);

    // T M10.10: per-session CRDT replica state. Bootstrapped by
    // `InstanceMessage::BufferSnapshot` messages routed in the
    // drain loop below; consumed by the optimistic-apply predicate
    // wired in Day 3.
    #[cfg(feature = "crdt")]
    let mut buffer_mirror = crate::buffer_mirror::BufferMirror::new(assigned_id);

    // T M11.6 — daemon-side input-dispatcher idleness. `false` =
    // pessimistic default before the first `DispatchIdle` arrives;
    // the daemon emits an initial value once per attach so this
    // converges to the true state on the first frame. While `false`,
    // every keystroke round-trips as `FrontendEvent::Key` regardless
    // of optimistic-apply eligibility — this is the fix for the
    // M10.10-era latent bug where chars typed into a minibuffer
    // prompt were optimistically applied to the previously-active
    // document.
    #[cfg(feature = "crdt")]
    let mut dispatch_idle = false;

    // Closure-and-call: any `return` from this closure still falls
    // through to `kick()` and `reader_handle.join()` below. Without
    // this wrapping a writer-side IO error inside the loop would skip
    // the wind-down.
    let result: Result<PumpEnd, AttachError> = (|| {
        // A write that fails on the socket is the same loss as its EOF, and may
        // come first, which the writer's thread says on the pump's next turn. Anything
        // else a frame says (a paste past its limit, encoded here) is not, and ends the
        // session as it says.
        let lost_or_failed = |error: AttachError, bootstrapped: bool| match error {
            AttachError::Transport(TransportError::Io(_)) => Ok(PumpEnd::Lost(LostSession {
                bootstrapped,
                write_error: Some(error),
                cut_short: false,
                notice_stood: true,
                far_side_said: None,
            })),
            other => Err(other),
        };
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
                    #[cfg(feature = "crdt")]
                    Ok(InstanceMessage::DispatchIdle { idle }) => {
                        // T M11.6 — daemon's input-dispatcher state.
                        // Consumed *here only*: no frontend rendering
                        // depends on the value; the optimistic-apply
                        // gate reads `dispatch_idle` below.
                        dispatch_idle = idle;
                    }
                    Ok(msg) => batch.push(msg),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        reader_eof = true;
                        break;
                    }
                }
            }
            if !batch.is_empty() {
                // The session's first frame puts up the notice, and a frame that takes
                // the bottom row while something stands there has it painted again.
                let framed = batch
                    .iter()
                    .any(|message| matches!(message, InstanceMessage::CellDelta { .. }));
                let mut takes_row = framed && takes_bottom_row(&batch, frontend.size());
                if framed {
                    bootstrapped = true;
                    if let Some(first) = notice.take() {
                        bottom.standing = Some(StandingNotice::new(first));
                        takes_row = true;
                    }
                }
                let over = bottom.wanted(frontend.size().cols).filter(|_| takes_row);
                let presented = match over {
                    Some(text) => {
                        bottom.painted = Some(text);
                        frontend.present_under_notice(&batch, text)
                    }
                    None => frontend.present_messages(&batch),
                };
                if let Err(e) = presented {
                    return Err(AttachError::Terminal(e));
                }
            }
            if let Some(reason) = goodbye {
                return Ok(PumpEnd::Over(Some(reason)));
            }
            if reader_eof {
                return Ok(PumpEnd::Lost(LostSession {
                    bootstrapped,
                    write_error: None,
                    cut_short: false,
                    notice_stood: true,
                    far_side_said: None,
                }));
            }
            if let Some(error) = writer.take_failure() {
                return lost_or_failed(
                    AttachError::Transport(TransportError::Io(error)),
                    bootstrapped,
                );
            }
            bottom.daemon_says(&writer.state(), answers.unanswered_for());
            bottom.time_passed();
            if let Err(e) = bottom.settle(frontend) {
                return Err(AttachError::Terminal(e));
            }

            let event = match frontend.poll_event(Duration::from_millis(50)) {
                Ok(opt) => opt,
                Err(e) => return Err(AttachError::Terminal(e)),
            };
            let Some(ev) = event else {
                continue;
            };

            if matches!(ev, Event::Key(_)) {
                bottom.key_arrived();
                let _ = bottom.settle(frontend);
            }

            if is_detach_key(&ev) {
                // Behind whatever is queued, and it may never go: the wind-down's kick is
                // what ends a write that is not moving.
                let _ = writer.send(&FrontendEvent::Detach(assigned_id));
                return Ok(PumpEnd::Over(None));
            }

            // Past the cap further input goes nowhere, before the optimistic path has
            // applied it to the mirror. A resize is not input: it is what the daemon next
            // paints to, and it is small.
            if writer.full() && !matches!(ev, Event::Resize(..)) {
                writer.input_dropped();
                continue;
            }
            if is_typed(&ev) {
                answers.input_sent();
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
                // T M11.6 — gate on daemon idleness. When the daemon
                // would intercept this key (minibuffer prompt active
                // or dispatcher holds a pending prefix), force the
                // keystroke to round-trip via `FrontendEvent::Key`
                // regardless of its optimistic-apply classification.
                // The orchestrator below still runs for non-key
                // events; only the Press/Repeat→CrdtOp path is gated.
                if !dispatch_idle {
                    false
                } else if matches!(k.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
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
                    if let Err(e) = writer.send(&frontend_event) {
                        return lost_or_failed(e, bootstrapped);
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

            if !optimistic_handled {
                let mut frame = Vec::new();
                if let Err(e) = forward_event(&mut frame, &ev, assigned_id, frontend.size()) {
                    return lost_or_failed(e, bootstrapped);
                }
                writer.push(frame);
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
    // 1. `close_within` — the writer finishes what is queued, `Detach`
    //    included, and drops its half of the protocol stream, on its
    //    own thread, where this used to be a `drop(writer)` here. For
    //    transports where the writer is the only handle to that half
    //    (SSH `ChildStdin`), this triggers an EOF cascade that ends with
    //    the reader thread seeing EOF naturally. For the local-socket
    //    transport, dropping one of three `UnixStream` clones is a no-op
    //    (other clones keep the FD alive); the reader needs `kick` to
    //    wake. Only a write that is not moving outlasts the grace.
    //
    // 2. `kick()` — wake the reader thread, and a writer still in a
    //    write, by any means necessary (per the kick contract). For
    //    local-socket: `shutdown(Both)`. For SSH: a watchdog that
    //    SIGTERMs the child if the EOF cascade hasn't reached the reader
    //    within the watchdog's grace period, and SIGKILLs one that
    //    outlasts that, which closes the pipe the writer is blocked on.
    //
    // 3. `join` — wait for both threads to exit, up to the one deadline,
    //    past which something `ssh` left behind holds a pipe and they are
    //    left behind.
    //
    // The close-before-kick ordering is what lets SSH's clean-detach
    // path produce exit code 0: writer close starts the cascade,
    // kick is best-effort backup, the cascade typically completes
    // before kick has any effect.
    let deadline = Instant::now() + WIND_DOWN_DEADLINE;
    writer.close_within(WRITER_CLOSE_GRACE);
    kick();
    let _ = join_within(
        writer.thread,
        deadline.saturating_duration_since(Instant::now()),
    );
    let cut_short = join_within(
        reader_handle,
        deadline.saturating_duration_since(Instant::now()),
    )
    .unwrap_or(false);

    let notice_stood = bottom.notice_stood();
    let far_side_said = said_in(&far_side_text);
    result.map(|end| match end {
        PumpEnd::Lost(lost) => PumpEnd::Lost(LostSession {
            cut_short,
            notice_stood,
            far_side_said,
            ..lost
        }),
        over @ PumpEnd::Over(_) => over,
    })
}

/// Whether a batch paints on a terminal's bottom row, as a resync always may.
fn takes_bottom_row(batch: &[InstanceMessage], size: CellSize) -> bool {
    let bottom = size.rows.saturating_sub(1);
    batch.iter().any(|message| match message {
        InstanceMessage::CellDelta { spans, full_grid } => {
            *full_grid
                || spans
                    .iter()
                    .any(|span| span.start.row == bottom && !span.cells.is_empty())
        }
        _ => false,
    })
}

/// Whether `ev` is something typed, pasted or clicked, which a user waits on an answer to:
/// the input [`Answers`] reads. A key's release is nothing sent, and a mouse's motion or
/// release, a focus change or a resize is not something a user waits on.
fn is_typed(ev: &Event) -> bool {
    use crossterm::event::MouseEventKind;
    match ev {
        Event::Key(key) => matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat),
        Event::Paste(_) => true,
        Event::Mouse(mouse) => !matches!(
            mouse.kind,
            MouseEventKind::Moved | MouseEventKind::Up(_) | MouseEventKind::Drag(_)
        ),
        Event::Resize(..) | Event::FocusGained | Event::FocusLost => false,
    }
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
//
// Returns whether the socket ended partway through a frame, which the channel's
// close cannot say.
//
// A frame's length past the cap is text from the far side, not a frame: `tx` goes first,
// so the pump goes on to its end and its kick, and what follows those four bytes is read
// into `text` until the channel ends.
#[allow(clippy::needless_pass_by_value)]
fn run_reader(
    mut reader: Box<dyn Read + Send>,
    tx: mpsc::Sender<InstanceMessage>,
    text: &Mutex<Vec<u8>>,
    last_frame: &Mutex<Instant>,
) -> bool {
    loop {
        match read_message::<InstanceMessage>(&mut reader) {
            Ok(msg) => {
                // Here and not where the pump drains it, since frames the daemon wrote
                // before it read a key wait in the channel behind it and answer nothing.
                *last_frame.lock().expect("the last frame's time") = Instant::now();
                if tx.send(msg).is_err() {
                    return false;
                }
            }
            Err(TransportError::CutShort) => return true,
            Err(TransportError::FrameTooLarge { len }) => {
                drop(tx);
                read_far_side_text(len, &mut reader, text);
                return false;
            }
            Err(_) => return false,
        }
    }
}

/// `len` was read as a frame's length and is past the cap: those four bytes and what comes
/// after them, into `text` as they arrive, so that what a far side said before something
/// held its channel open past the wait can still be read, up to [`SSH_STDERR_TAIL_CAP`].
fn read_far_side_text(len: usize, reader: &mut impl Read, text: &Mutex<Vec<u8>>) {
    let first = u32::try_from(len).unwrap_or(u32::MAX).to_be_bytes();
    text.lock()
        .expect("the far side's text")
        .extend_from_slice(&first);
    let mut buf = [0u8; 1024];
    loop {
        let n = match reader.read(&mut buf) {
            Ok(n) if n > 0 => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            _ => return,
        };
        let mut held = text.lock().expect("the far side's text");
        held.extend_from_slice(&buf[..n]);
        if held.len() >= SSH_STDERR_TAIL_CAP {
            return;
        }
    }
}

/// One line of what is in `text`, if anything is.
fn said_in(text: &Mutex<Vec<u8>>) -> Option<String> {
    let held = text.lock().expect("the far side's text");
    let line = far_side_line(&held);
    (!line.is_empty()).then_some(line)
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
/// container scenarios), the error is dropped. A child that outlasts it has a
/// SIGKILL [`SSH_KILL_AFTER_TERM`] later, and `reaped` says it is gone before either,
/// so neither reaches a pid that may be another process's by then. The reader and writer
/// threads end by the cascade or the signal, or are left behind at [`WIND_DOWN_DEADLINE`].
fn ssh_kick(child_pid: u32, reaped: Arc<AtomicBool>) -> Box<dyn FnOnce() + Send> {
    Box::new(move || {
        thread::spawn(move || {
            let reaped_within = |within: Duration| {
                let deadline = Instant::now() + within;
                while Instant::now() < deadline {
                    if reaped.load(Ordering::SeqCst) {
                        return true;
                    }
                    thread::sleep(Duration::from_millis(20));
                }
                reaped.load(Ordering::SeqCst)
            };
            // Cast to nix's `Pid` (an `i32` newtype). On Linux the
            // PID space fits in `i32`; the cast is lossless for any
            // real process ID.
            #[allow(clippy::cast_possible_wrap)]
            let pid = nix::unistd::Pid::from_raw(child_pid as i32);
            if reaped_within(SSH_KICK_WATCHDOG) {
                return;
            }
            let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM);
            if reaped_within(SSH_KILL_AFTER_TERM) {
                return;
            }
            let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGKILL);
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
    let mut unbootstrapped: u32 = 0;
    let mut memory = SessionMemory::default();
    let mut dying = DyingDaemons::default();
    let mut iteration: u32 = 0;

    let result: Result<Option<GoodbyeReason>, AttachError> = loop {
        // Pre-attempt overlay (skipped on the very first iteration —
        // no Frontend yet, and "reconnecting" would lie about state).
        // The chosen banner depends on whether we're reconnecting
        // mid-session (Frontend is up) or still trying to land the
        // first handshake (Frontend never came up).
        if iteration > 0 {
            if let Some(f) = frontend_slot.as_mut() {
                let _ = f.draw_status_overlay(banner_remote_reconnect());
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
        let mut ended = SessionEnded::default();
        let verdict = attempt_session(
            &mut frontend_slot,
            &target,
            initial_handshake_complete,
            &mut ended,
            &mut memory,
        );
        let loss = ended.loss;
        let mut keeps_dying = false;
        if let Some(lost) = loss {
            memory.lost(
                lost.bootstrapped && lost.notice_stood,
                lost.notice(&verdict),
            );
            keeps_dying = dying.session_lost(memory.reading, memory.session_lived());
        }
        let said = ended.far_side_said.take();
        let verdict =
            match remote_verdict(verdict, loss, &mut unbootstrapped, &target, said.as_deref()) {
                ReconnectVerdict::Reconnect { .. } if keeps_dying => {
                    ReconnectVerdict::ExitProtocolError {
                        error: AttachError::NewDaemonsKeepDying {
                            stderr: FarSideStderr::of(&target),
                            attempts: dying.in_a_row(),
                        },
                    }
                }
                verdict => verdict,
            };

        match verdict {
            ReconnectVerdict::ExitClean => {
                if let Some(f) = frontend_slot.as_mut() {
                    let _ = f.clear_status_overlay();
                }
                break Ok(ended.goodbye);
            }
            ReconnectVerdict::ExitProtocolError { error }
            | ReconnectVerdict::ExitPolicy { error } => {
                break Err(error);
            }
            ReconnectVerdict::Reconnect { error: last_error } => {
                // Mid-session disconnect: indefinite retry. The backoff
                // schedule walks the curve across failed attempts and
                // plateaus at 30s, matching mosh, and starts again after a
                // session that reached its first frame, as the local route's
                // does: that loss is an event of its own.
                if loss.is_some_and(|lost| lost.bootstrapped) {
                    backoff.reset();
                }
                // A loss is usually a stop, and a stop may have left the
                // terminal cooked, as on the local route.
                if let (Some(_), Some(f)) = (loss, frontend_slot.as_mut()) {
                    let _ = f.retake_terminal();
                }
                let delay = backoff.next_delay();
                let words = said.or_else(|| last_error.far_side_words());
                match sleep_with_countdown(frontend_slot.as_mut(), delay, words.as_deref()) {
                    SleepOutcome::Cancelled => break Ok(None),
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
                match sleep_with_countdown(frontend_slot.as_mut(), delay, None) {
                    SleepOutcome::Cancelled => break Ok(None),
                    SleepOutcome::Elapsed => {}
                }
            }
        }

        iteration = iteration.saturating_add(1);
    };

    // Tear down raw mode (if engaged) before main.rs's eprintln.
    drop(frontend_slot);

    said_goodbye(result?)
}

/// How a session that reached its pump ended, for the loop deciding what comes next.
#[derive(Debug, Default)]
struct SessionEnded {
    /// It was lost, and what it had reached; `None` for one that never got as far, and
    /// for one that ended on purpose.
    loss: Option<SessionLoss>,
    /// What its daemon's `Goodbye` said.
    goodbye: Option<GoodbyeReason>,
    /// What the far side said where a frame was expected, after its `Hello`.
    far_side_said: Option<String>,
}

/// What a session that got as far as its pump and lost its channel came to, for
/// [`remote_verdict`]; `None` for one that never did, such as a network that is
/// down, and for one that ended on purpose.
#[derive(Clone, Copy, Debug)]
struct SessionLoss {
    /// Whether its first frame arrived.
    bootstrapped: bool,
    /// Whether the channel ended partway through a frame.
    cut_short: bool,
    /// Whether the notice it had, if it had one, stood its floor.
    notice_stood: bool,
    /// How its `ssh` ended.
    exit: SshExit,
}

impl SessionLoss {
    /// What the next session says of this one, given `verdict`, the attempt's
    /// own: the bridge's clean exit is the daemon closing its socket, and
    /// anything else an `ssh` that failed.
    fn notice(self, verdict: &ReconnectVerdict) -> &'static str {
        if self.cut_short {
            NOTICE_FRESH_VIEW_CUT_SHORT
        } else if matches!(verdict, ReconnectVerdict::ExitClean) {
            NOTICE_FRESH_VIEW
        } else {
            NOTICE_FRESH_VIEW_LINK
        }
    }
}

/// E10r, the owner's ruling of 2026-11-06 --- what a lost session makes of the
/// SSH route's verdict, by what each failure costs the other frontends.
///
/// The real bridge exits 0 when the daemon closes its socket
/// (`run_bridge_connected`), so a session that was lost and an end the frontend
/// meant read alike by the exit, and only `loss` tells them: `None` is a `Goodbye`
/// or F12, which end the route, or a daemon never reached, which costs nobody
/// anything and keeps backing off with no limit. A session that had its first
/// frame and was then lost has paid its one hold-up, so it reconnects, whatever
/// its end. A session cut before its first frame, a link past the floor, costs the
/// others the bound on every try, and backoff alone would be 2 s in every 32 with
/// no end, so sessions in a row ending that way stop at [`HANDSHAKE_RETRY_CAP`]. The last
/// one says which: a frame cut short is the bound, an `ssh` that exited 0 is a daemon that
/// closed its socket, as one that died does, and that names where its stderr is, and any
/// other exit is `ssh`'s own, with no daemon to name.
fn remote_verdict(
    verdict: ReconnectVerdict,
    loss: Option<SessionLoss>,
    unbootstrapped: &mut u32,
    target: &AttachTarget,
    said: Option<&str>,
) -> ReconnectVerdict {
    let Some(loss) = loss else {
        return verdict;
    };
    if loss.bootstrapped {
        *unbootstrapped = 0;
    } else {
        *unbootstrapped = unbootstrapped.saturating_add(1);
    }
    match verdict {
        permanent @ (ReconnectVerdict::ExitProtocolError { .. }
        | ReconnectVerdict::ExitPolicy { .. }) => permanent,
        _ if *unbootstrapped >= HANDSHAKE_RETRY_CAP => ReconnectVerdict::ExitProtocolError {
            error: if loss.cut_short || loss.exit == SshExit::Clean {
                AttachError::SessionsDropped {
                    attempts: *unbootstrapped,
                    cut_short: loss.cut_short,
                    far_side: (!loss.cut_short).then(|| FarSideStderr::of(target)),
                }
            } else {
                AttachError::SshEndedBeforeFirstFrame {
                    host: route_host(target),
                    attempts: *unbootstrapped,
                    exit: loss.exit,
                    said: said.map(str::to_owned),
                }
            },
        },
        ReconnectVerdict::ExitClean => ReconnectVerdict::Reconnect {
            error: AttachError::Transport(TransportError::Eof),
        },
        ReconnectVerdict::Reconnect { error } | ReconnectVerdict::ReconnectHandshake { error } => {
            ReconnectVerdict::Reconnect { error }
        }
    }
}

/// Banner painted while a session-reconnect attempt is in flight.
///
/// Used when the Frontend is already up (the user has seen the
/// editor render at least once on this `run_attach_ssh` invocation).
/// No counter — session reconnects are unbounded.
fn banner_session_reconnect() -> &'static str {
    "[pmacs reconnecting...]"
}

/// [`banner_session_reconnect`] on the SSH route, whose wait lends the terminal, so
/// `C-c` reaches it as a signal and `F12` reaches nothing.
fn banner_remote_reconnect() -> &'static str {
    "[pmacs reconnecting... Ctrl-C to exit]"
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

/// [`banner_disconnected_countdown`] saying why, on a pane `cols` wide: the far side's
/// `words` first, fit by [`fit_far_side_words`] so that the countdown stays on the pane. A
/// pane too narrow for any of them has the banner alone.
fn banner_countdown(remaining: Duration, words: Option<&str>, cols: u32) -> String {
    use unicode_width::UnicodeWidthStr as _;
    let plain = banner_disconnected_countdown(remaining);
    let Some(words) = words else {
        return plain;
    };
    let secs = remaining.as_secs() + u64::from(remaining.subsec_millis() > 0);
    let tail = format!(" — reconnecting in {secs}s — Ctrl-C to exit]");
    let room = (cols as usize).saturating_sub(tail.width() + 1);
    if room < 8 {
        return plain;
    }
    format!("[{}{tail}", fit_far_side_words(words, room))
}

/// A far side's `words` in `room` columns of a bottom row, measured as a terminal draws
/// them (E10r's review 3, ruled 2026-11-08). U+FE0F goes first, since after a symbol whose
/// width is one it asks for an emoji two columns wide, which tmux's default and most
/// terminals draw, so a banner counted one column short ran past the row's end and
/// scrolled the screen at each second of a countdown (`crate::frontend` paints with it
/// gone too). Words too long for the room keep the clause before their first `;` or `(`,
/// where the reason is and before its detail, and of that as much of its end as fits: so
/// a directory the daemon refuses still reads `is too permissive` at 80 columns, where
/// keeping the start kept the path and lost the reason.
fn fit_far_side_words(words: &str, room: usize) -> String {
    use unicode_width::UnicodeWidthChar as _;
    const CUT: &str = "...";
    let width = |text: &str| -> usize { text.chars().map(|c| c.width().unwrap_or(0)).sum() };
    let words: String = words
        .chars()
        .filter(|c| *c != crate::frontend::EMOJI_PRESENTATION)
        .collect();
    if width(&words) <= room {
        return words;
    }
    let clause = words
        .split([';', '('])
        .next()
        .map(str::trim_end)
        .filter(|clause| !clause.is_empty())
        .unwrap_or(&words);
    if width(clause) <= room {
        return clause.to_owned();
    }
    let mut used = CUT.len();
    let mut kept: Vec<char> = Vec::new();
    for c in clause.chars().rev() {
        let w = c.width().unwrap_or(0);
        if used + w > room {
            break;
        }
        used += w;
        kept.push(c);
    }
    kept.reverse();
    format!("{CUT}{}", kept.into_iter().collect::<String>())
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

/// Sleep for `total` while painting a per-second countdown banner, saying `words` if the
/// far side left any.
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
fn sleep_with_countdown(
    frontend: Option<&mut Frontend>,
    total: Duration,
    words: Option<&str>,
) -> SleepOutcome {
    let Some(f) = frontend else {
        // No Frontend — no overlay, no input pump. Plain sleep.
        thread::sleep(total);
        return SleepOutcome::Elapsed;
    };

    let deadline = Instant::now() + total;

    // Initial paint so the banner appears immediately, not after the
    // first poll tick.
    let initial_banner = banner_countdown(total, words, f.size().cols);
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
            let banner = banner_countdown(remaining, words, f.size().cols);
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
    ended: &mut SessionEnded,
    memory: &mut SessionMemory,
) -> ReconnectVerdict {
    let raw_result = run_one_session(frontend_slot, target, ended, memory);
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
    (protocol_reader, stderr_handle, stderr_tail): ProtocolChannel,
    frontend: &mut Frontend,
    assigned_frontend_id: FrontendId,
    ended: &mut SessionEnded,
    notice: Option<Notice>,
) -> Result<(), AttachError> {
    let pid = child.id();
    let reaped = Arc::new(AtomicBool::new(false));
    let io = AttachIo {
        reader: protocol_reader,
        writer: Box::new(child_stdin),
        kick: ssh_kick(pid, Arc::clone(&reaped)),
    };
    let pump_end = run_attach_pair(io, frontend, assigned_frontend_id, notice);
    let over = matches!(pump_end, Ok(PumpEnd::Over(_)));
    let pump_result = pump_end.and_then(|end| match end {
        PumpEnd::Over(reason) => {
            ended.goodbye = reason;
            Ok(())
        }
        PumpEnd::Lost(lost) => {
            ended.loss = Some(SessionLoss {
                bootstrapped: lost.bootstrapped,
                cut_short: lost.cut_short,
                notice_stood: lost.notice_stood,
                exit: SshExit::Clean,
            });
            ended.far_side_said = lost.far_side_said;
            lost.write_error.map_or(Ok(()), Err)
        }
    });
    attach_debug(format!(
        "attach pump exited: {:?}",
        pump_result.as_ref().err()
    ));
    // The kick's SIGKILL bounds this wait, the child being this process's own.
    let exit_status = child.wait().ok();
    reaped.store(true, Ordering::SeqCst);
    if let Some(loss) = ended.loss.as_mut() {
        loss.exit = SshExit::of(exit_status);
    }
    let _ = join_within(stderr_handle, SSH_TERM_GRACE);
    // A session ended on purpose has ended, whatever `ssh` says after it: its exit is
    // read and ignored, since an `ssh` slower to leave than the kick's watchdog exits
    // on its SIGTERM, and this does not return before it does.
    if over {
        return Ok(());
    }
    let stderr_text = drain_stderr_tail(&stderr_tail);
    classify_ssh_exit(pump_result, exit_status, stderr_text)
}

/// A session's `ssh`, its channels, and the `Hello` it said.
struct OpenSession {
    child: Child,
    child_stdin: std::process::ChildStdin,
    channel: ProtocolChannel,
    hello: Hello,
}

/// What `C-c` and `C-\` do while a reconnect waits for its `Hello`: with the terminal
/// lent they are signals, to this process and to `ssh` beside it. The handlers are
/// installed once and stay, so outside a wait they take the default action, as a signal
/// from elsewhere did before this existed; raw mode raises none from the keyboard.
struct HelloWaitSignals {
    pressed: Arc<AtomicBool>,
    outside_a_wait: Arc<AtomicBool>,
}

impl HelloWaitSignals {
    /// The signals, armed for one wait.
    fn armed() -> std::io::Result<&'static Self> {
        use signal_hook::consts::{SIGINT, SIGQUIT};
        use signal_hook::flag::{register, register_conditional_default};
        static SIGNALS: OnceLock<Result<HelloWaitSignals, std::io::ErrorKind>> = OnceLock::new();
        let signals = SIGNALS.get_or_init(|| {
            let signals = Self {
                pressed: Arc::new(AtomicBool::new(false)),
                outside_a_wait: Arc::new(AtomicBool::new(true)),
            };
            for signal in [SIGINT, SIGQUIT] {
                register_conditional_default(signal, Arc::clone(&signals.outside_a_wait))
                    .and_then(|_| register(signal, Arc::clone(&signals.pressed)))
                    .map_err(|error| error.kind())?;
            }
            Ok(signals)
        });
        let signals = signals
            .as_ref()
            .map_err(|kind| std::io::Error::from(*kind))?;
        signals.pressed.store(false, Ordering::SeqCst);
        signals.outside_a_wait.store(false, Ordering::SeqCst);
        Ok(signals)
    }

    fn pressed(&self) -> bool {
        self.pressed.load(Ordering::SeqCst)
    }

    /// The wait is over. Whether one was pressed in it.
    fn disarm(&self) -> bool {
        self.outside_a_wait.store(true, Ordering::SeqCst);
        self.pressed()
    }
}

/// How a reconnect's wait for `Hello` came out.
enum HelloWait {
    Heard(Hello, Box<dyn Read + Send>),
    Failed(TransportError, Option<Box<dyn Read + Send>>),
    Cancelled,
    TimedOut(Duration),
}

/// The deadline, or what a test set in its place.
fn reconnect_hello_deadline() -> Duration {
    std::env::var(PMACS_TEST_RECONNECT_HELLO_MS)
        .ok()
        .and_then(|ms| ms.parse().ok())
        .map_or(RECONNECT_HELLO_DEADLINE, Duration::from_millis)
}

/// Wait for a reconnect's `Hello`, reading no key: the terminal is lent, and a byte
/// taken from it could be a passphrase's. The read is on a thread of its own, which is
/// left behind if it never returns, since something `ssh` started may hold the pipe.
fn wait_for_hello(mut reader: Box<dyn Read + Send>, signals: &HelloWaitSignals) -> HelloWait {
    const TICK: Duration = Duration::from_millis(20);
    let after = reconnect_hello_deadline();
    let deadline = Instant::now() + after;
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let said = read_message::<Hello>(&mut reader);
        let _ = tx.send((said, reader));
    });
    loop {
        if signals.pressed() {
            return HelloWait::Cancelled;
        }
        match rx.recv_timeout(TICK) {
            Ok((Ok(hello), reader)) => return HelloWait::Heard(hello, reader),
            Ok((Err(error), reader)) => {
                // `ssh` dying of the same `C-c` may be heard first.
                thread::sleep(TICK);
                return if signals.pressed() {
                    HelloWait::Cancelled
                } else {
                    HelloWait::Failed(error, Some(reader))
                };
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if Instant::now() >= deadline {
                    return HelloWait::TimedOut(after);
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return HelloWait::Failed(TransportError::Eof, None);
            }
        }
    }
}

/// Spawn `ssh` and hear its `Hello`. The first attempt (`signals` is `None`) reads it on
/// a cooked terminal, where `C-c` ends the process and a failure reaches the user
/// directly. A reconnect has lent the terminal, and waits with a deadline.
fn open_session(
    target: &AttachTarget,
    signals: Option<&HelloWaitSignals>,
) -> Result<OpenSession, AttachError> {
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
    let child_stdin = child
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
        signals.is_none(),
    );

    attach_debug("waiting for Hello from remote daemon bridge");
    let hello_wait_done = spawn_hello_watchdog();
    let heard = match signals {
        None => match read_message::<Hello>(&mut protocol_reader) {
            Ok(hello) => Ok((hello, protocol_reader)),
            Err(error) => Err((error.into(), Some(protocol_reader))),
        },
        Some(signals) => match wait_for_hello(protocol_reader, signals) {
            HelloWait::Heard(hello, reader) => Ok((hello, reader)),
            HelloWait::Failed(error, reader) => Err((error.into(), reader)),
            HelloWait::Cancelled => Err((AttachError::ReconnectCancelled, None)),
            HelloWait::TimedOut(after) => {
                Err((AttachError::ReconnectHelloTimedOut { after }, None))
            }
        },
    };
    hello_wait_done.store(true, Ordering::SeqCst);
    let (hello, protocol_reader) = match heard {
        Ok(heard) => heard,
        Err((error, reader)) => {
            attach_debug(format!("failed reading Hello: {error}"));
            return Err(no_hello_error(
                child,
                stderr_handle,
                stderr_tail,
                error,
                reader,
            ));
        }
    };
    attach_debug(format!(
        "received Hello: protocol_version={}, assigned_frontend_id={}",
        hello.protocol_version, hello.assigned_frontend_id.0
    ));
    Ok(OpenSession {
        child,
        child_stdin,
        channel: (protocol_reader, stderr_handle, stderr_tail),
        hello,
    })
}

/// [`open_session`] on a reconnect: the terminal lent for the wait and taken back after
/// it, whatever came of it, so the schedule's wait reads its `C-c` from a raw terminal.
fn reopen_session(
    frontend: &mut Frontend,
    target: &AttachTarget,
) -> Result<OpenSession, AttachError> {
    let signals = HelloWaitSignals::armed().map_err(AttachError::Terminal)?;
    if let Err(error) = frontend.lend_terminal() {
        signals.disarm();
        return Err(AttachError::Terminal(error));
    }
    let opened = open_session(target, Some(signals));
    let back = frontend.take_terminal_back();
    let pressed = signals.disarm();
    let opened = match (opened, pressed) {
        // Pressed as the `Hello` arrived.
        (Ok(open), true) => {
            let (_, stderr_handle, stderr_tail) = open.channel;
            Err(handshake_error_with_child(
                open.child,
                stderr_handle,
                stderr_tail,
                AttachError::ReconnectCancelled,
            ))
        }
        (opened, _) => opened,
    };
    back.map_err(AttachError::Terminal).and(opened)
}

/// Body of [`attempt_session`] expressed in raw `AttachError` so the
/// caller can fold the result through [`classify_for_reconnect`].
/// Splitting this out keeps the IO path linear and tests-friendly:
/// `attempt_session` is just classify + dispatch.
fn run_one_session(
    frontend_slot: &mut Option<Frontend>,
    target: &AttachTarget,
    ended: &mut SessionEnded,
    memory: &mut SessionMemory,
) -> Result<(), AttachError> {
    // Hello / AttachRequest handshake. On the FIRST attempt
    // (`frontend_slot` is `None`) raw mode is not engaged yet, so
    // failures here reach a cooked terminal via main.rs's eprintln.
    // On reconnect (`Some`) the Frontend is up and the M5.8c
    // overlay communicates state; handshake errors here are bubbled
    // up so the M5.8d loop can decide whether to retry.
    let OpenSession {
        child,
        mut child_stdin,
        channel: (protocol_reader, stderr_handle, stderr_tail),
        hello,
    } = match frontend_slot.as_mut() {
        None => open_session(target, None)?,
        Some(frontend) => reopen_session(frontend, target)?,
    };
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

    memory.heard(&hello);

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
    // pattern as the local-socket path above, including Stage 2B-3's
    // counter-offer: this path *spawns* the daemon from the running
    // executable, so the peer is always this same ladder rung and the
    // counter-offer is always accepted.
    let req = AttachRequest {
        protocol_version: crate::protocol::requested_protocol_version(hello.protocol_version),
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
        (protocol_reader, stderr_handle, stderr_tail),
        frontend,
        hello.assigned_frontend_id,
        ended,
        memory.notice(),
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
    let _ = end_the_child(&mut child);
    let _ = join_within(stderr_handle, SSH_TERM_GRACE);
    original_error
}

/// Best-effort tear-down of the child, and how it ended. SIGTERM unsticks any read
/// it might be doing on its end of our broken handshake, and one that
/// outlasts it has a SIGKILL. Errors are ignored — if the child has
/// already exited (e.g., remote `pmacs` not on PATH), `kill` returns
/// ESRCH and we move on. Something it started may hold its stderr open
/// past it, so the caller does not wait for that without a limit.
fn end_the_child(child: &mut Child) -> Option<std::process::ExitStatus> {
    #[allow(clippy::cast_possible_wrap)]
    let pid = nix::unistd::Pid::from_raw(child.id() as i32);
    let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM);
    let deadline = Instant::now() + SSH_TERM_GRACE;
    while matches!(child.try_wait(), Ok(None)) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    let _ = child.kill();
    child.wait().ok()
}

/// A `Hello` that did not come, for `error`, and what the far side's way of saying so was.
///
/// A frame length past the cap is text from the far side: a shell with no `pmacs` to run,
/// `ssh` itself, or the bridge's own error, since the protocol rides `ssh`'s stderr (#477).
/// So those bytes and what follows them are the far side's words, and the error says them
/// and how `ssh` ended, its 127 included, in place of a frame length. A channel that simply
/// ended says how `ssh` ended, if that was a failure. A far side still going after a moment
/// has said what it will, and is ended here as any failed handshake is.
fn no_hello_error(
    mut child: Child,
    stderr_handle: thread::JoinHandle<()>,
    stderr_tail: Arc<Mutex<VecDeque<u8>>>,
    error: AttachError,
    reader: Option<Box<dyn Read + Send>>,
) -> AttachError {
    use TransportError::{CutShort, Eof, FrameTooLarge};
    let len = match error {
        AttachError::Transport(FrameTooLarge { len }) => Some(len),
        AttachError::Transport(Eof | CutShort) => None,
        other => return handshake_error_with_child(child, stderr_handle, stderr_tail, other),
    };
    let text = Arc::new(Mutex::new(Vec::new()));
    let reading = len.zip(reader).map(|(len, mut reader)| {
        let text = Arc::clone(&text);
        thread::spawn(move || read_far_side_text(len, &mut reader, &text))
    });
    let deadline = Instant::now() + SSH_TERM_GRACE;
    while matches!(child.try_wait(), Ok(None)) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    let status = end_the_child(&mut child);
    if let Some(reading) = reading {
        let _ = join_within(reading, SSH_TERM_GRACE);
    }
    let _ = join_within(stderr_handle, SSH_TERM_GRACE);
    let code = status.and_then(|status| status.code());
    if let Some(words) = said_in(&text) {
        return AttachError::FarSideSaid { code, words };
    }
    if SshExit::of(status) == SshExit::Clean {
        return error;
    }
    AttachError::SshChildExited {
        code,
        stderr_tail: drain_stderr_tail(&stderr_tail),
    }
}

/// Join `handle` if it finishes within `within`, and leave it running if it does not.
fn join_within<T>(handle: thread::JoinHandle<T>, within: Duration) -> Option<T> {
    let deadline = Instant::now() + within;
    while !handle.is_finished() {
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(5));
    }
    handle.join().ok()
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
        assert_eq!(
            c.true_color,
            crate::color_depth::ColorDepth::detect_env()
                == crate::color_depth::ColorDepth::TrueColor
        );
        assert!(capabilities_for(crate::color_depth::ColorDepth::TrueColor).true_color);
        assert!(!capabilities_for(crate::color_depth::ColorDepth::Sixteen).true_color);
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
        let result = run_attach_pair(io, &mut frontend, FrontendId::LOCAL, None);

        daemon.join().expect("daemon thread");

        assert!(result.is_ok(), "pump exits Ok on Goodbye: got {result:?}");
        assert_eq!(frontend.presented.len(), 1, "exactly one message presented");
        assert!(matches!(
            frontend.presented[0],
            InstanceMessage::Cursor(None)
        ));
    }

    /// E10r fix round 1: the pump tells a session that ended on purpose from
    /// one whose socket ended, and says whether a lost one reached its first
    /// frame, which is what the routes decide on.
    #[test]
    fn the_pump_tells_a_goodbye_from_a_lost_socket() {
        let run = |write: fn(&mut UnixStream)| {
            let (mut daemon_side, frontend_side) = UnixStream::pair().expect("pair");
            let io = build_local_socket_io(frontend_side).expect("io");
            write(&mut daemon_side);
            drop(daemon_side);
            let mut frontend = FakeFrontend::default();
            run_attach_pair(io, &mut frontend, FrontendId::LOCAL, None).expect("no error")
        };
        assert!(matches!(
            run(
                |w| write_message(w, &InstanceMessage::Goodbye(GoodbyeReason::ShuttingDown))
                    .expect("write")
            ),
            PumpEnd::Over(Some(GoodbyeReason::ShuttingDown))
        ));
        assert!(matches!(
            run(|_| {}),
            PumpEnd::Lost(LostSession {
                bootstrapped: false,
                write_error: None,
                cut_short: false,
                notice_stood: true,
                far_side_said: None,
            })
        ));
        assert!(matches!(
            run(|w| write_message(w, &InstanceMessage::Cursor(None)).expect("write")),
            PumpEnd::Lost(LostSession {
                bootstrapped: false,
                write_error: None,
                cut_short: false,
                notice_stood: true,
                far_side_said: None,
            })
        ));
        assert!(matches!(
            run(|w| w.write_all(&100u32.to_be_bytes()).expect("write")),
            PumpEnd::Lost(LostSession {
                bootstrapped: false,
                write_error: None,
                cut_short: true,
                notice_stood: true,
                far_side_said: None,
            })
        ));
        assert!(matches!(
            run(|w| write_message(
                w,
                &InstanceMessage::CellDelta {
                    spans: Vec::new(),
                    full_grid: true,
                },
            )
            .expect("write")),
            PumpEnd::Lost(LostSession {
                bootstrapped: true,
                write_error: None,
                cut_short: false,
                notice_stood: true,
                far_side_said: None,
            })
        ));
    }

    /// E10r fix round 1: what a connect finds decides whether the local route
    /// goes on. A daemon that is gone ends it; so does a path that is not a
    /// socket; the error says which.
    #[test]
    fn a_connect_that_finds_no_daemon_is_a_daemon_gone() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let size = CellSize::new(24, 80);
        let missing = dir.path().join("missing.sock");
        match connect_again(&missing, size, &mut SessionMemory::default()) {
            Err(AttachError::DaemonGone { socket, found }) => {
                assert_eq!(socket, missing);
                assert_eq!(found, "no socket is there");
            }
            other => panic!("expected DaemonGone: {:?}", other.err()),
        }
        let stale = dir.path().join("stale.sock");
        let Some(listener) = bind_or_skip(&stale) else {
            return;
        };
        drop(listener);
        match connect_again(&stale, size, &mut SessionMemory::default()) {
            Err(AttachError::DaemonGone { found, .. }) => {
                assert_eq!(found, "its socket refused the connection");
            }
            other => panic!("expected DaemonGone: {:?}", other.err()),
        }
        let file = dir.path().join("file");
        std::fs::write(&file, b"not a socket").expect("write");
        match connect_again(&file, size, &mut SessionMemory::default()) {
            Err(AttachError::DaemonGone { found, .. }) => {
                assert_eq!(found, "what is there is not a socket");
            }
            other => panic!("expected DaemonGone: {:?}", other.err()),
        }
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
        let result = run_attach_pair(io, &mut frontend, FrontendId::LOCAL, None);

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
        let result = run_attach_pair(io, &mut frontend, FrontendId::LOCAL, None);

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
        let no_text = Arc::new(Mutex::new(Vec::new()));
        let no_frame = Mutex::new(Instant::now());
        let reader_handle =
            thread::spawn(move || run_reader(signaling_reader, tx, &no_text, &no_frame));

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
                    env!("CARGO_PKG_VERSION"),
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

    // -----------------------------------------------------------------
    // E10r fix round 3, H2 --- the writer's thread
    // -----------------------------------------------------------------

    /// A frontend with a script of events, which records what stands on its bottom row.
    #[derive(Default)]
    struct ScriptedFrontend {
        events: VecDeque<Event>,
        row: Vec<Option<String>>,
    }

    impl AttachPumpFrontend for ScriptedFrontend {
        fn present_messages(&mut self, _msgs: &[InstanceMessage]) -> std::io::Result<()> {
            Ok(())
        }
        fn poll_event(&mut self, timeout: Duration) -> std::io::Result<Option<Event>> {
            let event = self.events.pop_front();
            if event.is_none() {
                thread::sleep(timeout.min(Duration::from_millis(5)));
            }
            Ok(event)
        }
        fn size(&self) -> CellSize {
            CellSize::new(24, 80)
        }
        fn present_under_notice(
            &mut self,
            _msgs: &[InstanceMessage],
            text: &str,
        ) -> std::io::Result<()> {
            self.row.push(Some(text.to_owned()));
            Ok(())
        }
        fn clear_notice(&mut self) -> std::io::Result<()> {
            self.row.push(None);
            Ok(())
        }
    }

    fn press(code: KeyCode) -> Event {
        use crossterm::event::{KeyEvent as CtKey, KeyEventState, KeyModifiers as CtMods};
        Event::Key(CtKey {
            code,
            modifiers: CtMods::empty(),
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        })
    }

    /// A writer that fails every write, as a socket gone does.
    struct BrokenWriter;

    impl Write for BrokenWriter {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// What must not move: a failed write still reaches `lost_or_failed` by way of the
    /// pump, a lost session carrying its error, and it now does so from another thread.
    #[test]
    fn a_failed_write_is_a_lost_session_by_way_of_the_pump() {
        let (daemon_side, frontend_side) = UnixStream::pair().expect("pair");
        // A pump that never hears of the failure ends here, read as a loss with no error.
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(10));
            drop(daemon_side);
        });
        let io = AttachIo {
            writer: Box::new(BrokenWriter),
            ..build_local_socket_io(frontend_side).expect("io")
        };
        let mut frontend = ScriptedFrontend::default();
        frontend.events.push_back(press(KeyCode::Char('a')));
        let end = run_attach_pair(io, &mut frontend, FrontendId::LOCAL, None).expect("no error");
        assert!(
            matches!(
                end,
                PumpEnd::Lost(LostSession {
                    bootstrapped: false,
                    write_error: Some(AttachError::Transport(TransportError::Io(_))),
                    cut_short: false,
                    notice_stood: true,
                    far_side_said: None,
                })
            ),
            "{end:?}"
        );
    }

    /// What must not move: a paste past a frame's limit is not a lost socket. It ends the
    /// session as it says, encoded on the pump, with nothing written.
    #[test]
    fn a_paste_past_a_frame_ends_the_session_as_it_says() {
        let (_daemon_side, frontend_side) = UnixStream::pair().expect("pair");
        let io = build_local_socket_io(frontend_side).expect("io");
        let mut frontend = ScriptedFrontend::default();
        frontend
            .events
            .push_back(Event::Paste("x".repeat(MAX_FRAME_BYTES + 1)));
        let error = run_attach_pair(io, &mut frontend, FrontendId::LOCAL, None)
            .expect_err("a paste past the frame");
        assert!(
            matches!(
                error,
                AttachError::Transport(TransportError::FrameTooLarge { .. })
            ),
            "{error:?}"
        );
    }

    /// What a [`RecordingWriter`] saw, and when it went.
    #[derive(Default)]
    struct WriterRecord {
        bytes: Vec<u8>,
        dropped: bool,
    }

    /// A writer that keeps what it is given and says when it is dropped.
    struct RecordingWriter(Arc<Mutex<WriterRecord>>);

    impl Write for RecordingWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("record").bytes.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Drop for RecordingWriter {
        fn drop(&mut self) {
            self.0.lock().expect("record").dropped = true;
        }
    }

    /// What must not move, the function's primary correctness invariant: `F12` writes its
    /// `Detach`, and the writer is dropped, which is what starts the SSH route's clean
    /// detach, before the kick.
    #[test]
    fn detach_is_written_and_the_writer_dropped_before_the_kick() {
        let (_daemon_side, frontend_side) = UnixStream::pair().expect("pair");
        let reader = frontend_side.try_clone().expect("reader");
        let record = Arc::new(Mutex::new(WriterRecord::default()));
        let at_the_kick = Arc::new(Mutex::new(None));
        let seen = Arc::clone(&at_the_kick);
        let read_at_the_kick = Arc::clone(&record);
        let io = AttachIo {
            reader: Box::new(reader),
            writer: Box::new(RecordingWriter(Arc::clone(&record))),
            kick: Box::new(move || {
                let record = read_at_the_kick.lock().expect("record");
                *seen.lock().expect("seen") = Some((record.dropped, record.bytes.clone()));
                let _ = frontend_side.shutdown(Shutdown::Both);
            }),
        };
        let mut frontend = ScriptedFrontend::default();
        frontend.events.push_back(press(KeyCode::F(12)));
        let end = run_attach_pair(io, &mut frontend, FrontendId::LOCAL, None).expect("no error");
        assert!(matches!(end, PumpEnd::Over(None)), "{end:?}");
        let (dropped, bytes) = at_the_kick
            .lock()
            .expect("seen")
            .take()
            .expect("the kick fired");
        assert!(dropped, "the writer was still open at the kick");
        let said: FrontendEvent = read_message(&mut bytes.as_slice()).expect("a frame");
        assert!(matches!(said, FrontendEvent::Detach(_)), "{said:?}");
    }

    /// A writer that blocks until told, as a write into a full socket does, keeping what it
    /// is given from then on.
    struct HeldWriter {
        released: Arc<(Mutex<bool>, std::sync::Condvar)>,
        record: Arc<Mutex<WriterRecord>>,
    }

    impl Write for HeldWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let (released, wake) = &*self.released;
            let mut free = released.lock().expect("released");
            while !*free {
                free = wake.wait(free).expect("released");
            }
            self.record
                .lock()
                .expect("record")
                .bytes
                .extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    type Release = Arc<(Mutex<bool>, std::sync::Condvar)>;

    fn release(released: &Release) {
        *released.0.lock().expect("released") = true;
        released.1.notify_all();
    }

    /// A write that never returns holds nothing that reads the terminal, and the kick wakes
    /// it: `F12` ends the pump past it, within the wind-down, and the frames it held come
    /// after in the order they were sent.
    #[test]
    fn a_write_that_blocks_holds_no_key_and_the_kick_wakes_it() {
        let (_daemon_side, frontend_side) = UnixStream::pair().expect("pair");
        let reader = frontend_side.try_clone().expect("reader");
        let released: Release = Arc::default();
        let record = Arc::new(Mutex::new(WriterRecord::default()));
        let kicked = Arc::clone(&released);
        let io = AttachIo {
            reader: Box::new(reader),
            writer: Box::new(HeldWriter {
                released: Arc::clone(&released),
                record: Arc::clone(&record),
            }),
            kick: Box::new(move || {
                let _ = frontend_side.shutdown(Shutdown::Both);
                release(&kicked);
            }),
        };
        let mut frontend = ScriptedFrontend::default();
        frontend.events.extend([
            press(KeyCode::Char('a')),
            press(KeyCode::Char('b')),
            press(KeyCode::F(12)),
        ]);
        let start = Instant::now();
        let end = run_attach_pair(io, &mut frontend, FrontendId::LOCAL, None).expect("no error");
        assert!(matches!(end, PumpEnd::Over(None)), "{end:?}");
        assert!(
            start.elapsed() < WIND_DOWN_DEADLINE,
            "{:?}",
            start.elapsed()
        );
        let bytes = record.lock().expect("record").bytes.clone();
        let mut left = bytes.as_slice();
        let mut said = Vec::new();
        while !left.is_empty() {
            said.push(read_message::<FrontendEvent>(&mut left).expect("a whole frame"));
        }
        assert!(
            matches!(
                said.as_slice(),
                [
                    FrontendEvent::Key(_),
                    FrontendEvent::Key(_),
                    FrontendEvent::Detach(_)
                ]
            ),
            "in the order they were sent: {said:?}"
        );
    }

    /// How the writer says how it stands: nothing pending until a frame is, full at its
    /// cap, and each of those gone when the queue drains.
    #[test]
    fn the_writer_says_how_it_stands() {
        let released: Release = Arc::default();
        let record = Arc::new(Mutex::new(WriterRecord::default()));
        let mut writer = FrameWriter::spawn(Box::new(HeldWriter {
            released: Arc::clone(&released),
            record: Arc::clone(&record),
        }));
        writer.cap = 8;
        assert!(!writer.state().pending);
        assert!(!writer.full());
        writer.push(Vec::new());
        assert!(
            !writer.state().pending,
            "a key's release encodes to nothing"
        );
        writer.push(vec![1; 4]);
        assert!(writer.state().pending);
        assert!(!writer.full(), "under the cap a frame is taken");
        writer.push(vec![2; 100]);
        assert!(writer.full(), "at the cap further input is not");
        writer.input_dropped();
        assert!(writer.state().dropped);
        release(&released);
        let deadline = Instant::now() + Duration::from_secs(10);
        while writer.state().pending && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        let state = writer.state();
        assert!(!state.pending, "the queue drained");
        assert!(!state.dropped, "and what it said goes with it");
        assert!(!writer.full());
        writer.close_within(Duration::from_secs(10));
        writer.thread.join().expect("the thread");
        let record = record.lock().expect("record");
        assert_eq!(record.bytes.len(), 104, "every byte, in order");
        assert!(record.bytes[..4].iter().all(|b| *b == 1));
    }

    /// The owner's ruling: at least one largest frame, or a paste as large as a frame can
    /// be would be dropped at an empty queue.
    #[test]
    fn a_write_queue_holds_at_least_one_largest_frame() {
        assert_eq!(WRITE_QUEUE_CAP, MAX_FRAME_BYTES);
        assert_eq!(NO_ANSWER_NOTICE, Duration::from_secs(2));
    }

    /// The owner's words, character for character, and a narrow pane's that keep `F12`.
    #[test]
    fn a_daemon_not_answering_reads_as_ruled_at_every_width() {
        assert_eq!(
            NOTICE_NOT_ANSWERING.fitting(80),
            "[pmacs: the daemon is not answering; F12 detaches]"
        );
        assert_eq!(
            NOTICE_KEYS_DROPPED.fitting(80),
            "[pmacs: the daemon is not answering; keys dropped; F12 detaches]"
        );
        for notice in [NOTICE_NOT_ANSWERING, NOTICE_KEYS_DROPPED] {
            let narrow = notice.fitting(30);
            assert!(narrow.contains("F12"), "{narrow}");
            assert!(narrow.chars().count() <= 30, "{narrow}");
        }
    }

    /// The floor as ruled: a key during it takes the notice off at its end, one after it
    /// at once, and with none it stands however long, however many frames take its row.
    #[test]
    fn a_notice_comes_off_by_itself_only_if_a_key_arrived_during_its_floor() {
        let long_ago = Instant::now()
            .checked_sub(NOTICE_FLOOR)
            .expect("an instant that long ago");
        let standing = |since, keyed| {
            Some(StandingNotice {
                notice: NOTICE_NEW_DAEMON,
                since,
                keyed,
            })
        };
        let mut bottom = BottomRow {
            standing: standing(long_ago, false),
            ..BottomRow::default()
        };
        bottom.time_passed();
        assert!(
            bottom.standing.is_some(),
            "no key: it stands past its floor"
        );
        assert!(bottom.notice_stood());
        bottom.key_arrived();
        assert!(bottom.standing.is_none(), "the first key after its floor");

        let mut bottom = BottomRow {
            standing: standing(Instant::now(), false),
            ..BottomRow::default()
        };
        assert!(!bottom.notice_stood());
        bottom.key_arrived();
        assert!(bottom.standing.is_some(), "a key inside its floor");
        assert!(bottom.standing.is_some_and(|s| s.keyed));
        assert!(!bottom.notice_stood());

        let mut bottom = BottomRow {
            standing: standing(long_ago, true),
            ..BottomRow::default()
        };
        bottom.time_passed();
        assert!(
            bottom.standing.is_none(),
            "its floor over, a key having come"
        );
        assert!(bottom.notice_stood());
    }

    /// How the sentences sit: `not answering` stands over a new session's notice from a
    /// key's 2 s with no frame until the next frame, `keys dropped` over both until the queue
    /// drains, and the notice comes back if it still stands. The writer's own movement
    /// says nothing: a full transport with frames still coming is a daemon answering.
    #[test]
    fn the_daemons_sentences_stand_over_a_notice_and_give_it_back() {
        let mut bottom = BottomRow {
            standing: Some(StandingNotice::new(NOTICE_NEW_DAEMON)),
            ..BottomRow::default()
        };
        let mut frontend = ScriptedFrontend::default();
        bottom.settle(&mut frontend).expect("settle");
        let writer = |pending, dropped| WriterState { pending, dropped };
        bottom.daemon_says(&writer(true, false), Some(Duration::from_secs(1)));
        bottom.settle(&mut frontend).expect("settle");
        bottom.daemon_says(&writer(true, false), Some(NO_ANSWER_NOTICE));
        bottom.settle(&mut frontend).expect("settle");
        bottom.daemon_says(&writer(true, true), Some(NO_ANSWER_NOTICE));
        bottom.settle(&mut frontend).expect("settle");
        // A frame read with the queue not yet drained: `keys dropped` stays, as ruled.
        bottom.daemon_says(&writer(true, true), None);
        bottom.settle(&mut frontend).expect("settle");
        bottom.daemon_says(&writer(false, false), None);
        bottom.settle(&mut frontend).expect("settle");
        bottom.daemon_says(&writer(true, false), Some(NO_ANSWER_NOTICE));
        bottom.settle(&mut frontend).expect("settle");
        // The next frame takes it off, the queue still holding what it held.
        bottom.daemon_says(&writer(true, false), None);
        bottom.settle(&mut frontend).expect("settle");
        bottom.standing = None;
        bottom.settle(&mut frontend).expect("settle");
        let expect: Vec<Option<String>> = vec![
            Some(NOTICE_NEW_DAEMON.wide.to_owned()),
            Some(NOTICE_NOT_ANSWERING.wide.to_owned()),
            Some(NOTICE_KEYS_DROPPED.wide.to_owned()),
            Some(NOTICE_NEW_DAEMON.wide.to_owned()),
            Some(NOTICE_NOT_ANSWERING.wide.to_owned()),
            Some(NOTICE_NEW_DAEMON.wide.to_owned()),
            None,
        ];
        assert_eq!(frontend.row, expect);
    }

    /// A key starts the clock and the first frame read after it stops it; a frame read
    /// before it answers nothing, and keys after the first do not move the clock on.
    #[test]
    fn a_key_waits_for_the_first_frame_read_after_it() {
        let mut answers = Answers::new();
        assert_eq!(answers.unanswered_for(), None, "nothing sent");
        *answers.last_frame.lock().expect("time") = Instant::now();
        thread::sleep(Duration::from_millis(5));
        answers.input_sent();
        let first = answers.unanswered_since.expect("the key's time");
        assert!(
            answers.unanswered_for().is_some(),
            "a frame read before the key answers nothing"
        );
        thread::sleep(Duration::from_millis(5));
        answers.input_sent();
        assert_eq!(answers.unanswered_since, Some(first), "the oldest key's");
        thread::sleep(Duration::from_millis(5));
        assert!(
            answers
                .unanswered_for()
                .is_some_and(|waited| waited >= Duration::from_millis(10))
        );
        *answers.last_frame.lock().expect("time") = Instant::now();
        assert_eq!(answers.unanswered_for(), None, "a frame read after it");
        assert_eq!(answers.unanswered_since, None);
    }

    /// What starts the clock: a key pressed or held, a paste, a click or a wheel; not a
    /// release, a mouse's motion, a focus change or a resize.
    #[test]
    fn what_a_user_waits_on_is_what_is_typed_pasted_or_clicked() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        let mouse = |kind| {
            Event::Mouse(MouseEvent {
                kind,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            })
        };
        let mut release = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        assert!(is_typed(&press(KeyCode::Char('a'))));
        assert!(is_typed(&Event::Paste("x".to_owned())));
        assert!(is_typed(&mouse(MouseEventKind::Down(MouseButton::Left))));
        assert!(is_typed(&mouse(MouseEventKind::ScrollDown)));
        assert!(!is_typed(&Event::Key(release)));
        assert!(!is_typed(&mouse(MouseEventKind::Moved)));
        assert!(!is_typed(&mouse(MouseEventKind::Up(MouseButton::Left))));
        assert!(!is_typed(&Event::FocusLost));
        assert!(!is_typed(&Event::Resize(80, 24)));
    }

    /// What a far side says reaches a terminal as one line of text: its last with text and
    /// no control character, a frame's bytes behind it not taking its place, and no escape
    /// sequence whatever comes.
    #[test]
    fn a_far_sides_words_reach_a_terminal_as_a_line_of_text() {
        assert_eq!(
            far_side_line(b"bash: line 1: pmacs: command not found\n"),
            "bash: line 1: pmacs: command not found"
        );
        assert_eq!(
            far_side_line(b"Warning: added\r\nPermission denied (publickey).\r\n"),
            "Permission denied (publickey)."
        );
        assert_eq!(
            far_side_line(b"welcome to the host\n\x00\x00\x00\x10\x01\x02binary"),
            "welcome to the host"
        );
        assert_eq!(
            far_side_line(b"\x1b[31mred\x1b[0m \x1b]0;title\x07words\n"),
            "[31mred[0m ]0;titlewords"
        );
        assert_eq!(far_side_line(b""), "");
        assert_eq!(far_side_line(b"\n \n"), "");
        // Past the limit, on a character's boundary, as the old byte slice was not.
        let long = "é".repeat(FAR_SIDE_LINE_CHARS + 50);
        let line = far_side_line(long.as_bytes());
        assert_eq!(line, format!("...{}", "é".repeat(FAR_SIDE_LINE_CHARS)));
    }

    /// The countdown says why, clipped to the pane with the countdown still on it, and
    /// is what it was with nothing to say or no room to say it.
    #[test]
    fn the_countdown_says_what_the_far_side_said_clipped_to_the_pane() {
        use unicode_width::UnicodeWidthStr as _;
        let in_16 = Duration::from_secs(16);
        let plain = banner_disconnected_countdown(in_16);
        assert_eq!(banner_countdown(in_16, None, 80), plain);
        assert_eq!(banner_countdown(in_16, Some("anything"), 30), plain);
        assert_eq!(
            banner_countdown(in_16, Some("bash: not found"), 80),
            "[bash: not found — reconnecting in 16s — Ctrl-C to exit]"
        );
        let long = "x".repeat(300);
        let clipped = banner_countdown(in_16, Some(&long), 80);
        assert_eq!(clipped.width(), 80, "{clipped}");
        assert!(
            clipped.ends_with("— reconnecting in 16s — Ctrl-C to exit]"),
            "{clipped}"
        );
        // A wide character counts as what it takes on the pane.
        let wide = "字".repeat(100);
        assert!(banner_countdown(in_16, Some(&wide), 80).width() <= 80);
    }

    /// **Review 3's Low 1, its `p41b`:** a far side's emoji, a symbol and U+FE0F, which
    /// tmux's default draws two columns wide, counted one, so the countdown ran past the row
    /// and scrolled the screen once a second. Measured as drawn it fits, at 100 columns, by
    /// `UnicodeWidthStr`, which counts the sequence as two; at `e111983` it is wider.
    #[test]
    fn a_far_sides_emoji_keeps_the_countdown_on_its_row() {
        use unicode_width::UnicodeWidthStr as _;
        let in_16 = Duration::from_secs(16);
        let warning = "bash: warning: ⚠️ setlocale: LC_ALL: cannot change locale (en_US.UTF-8): \
                       No such file or directory, and this login script carries on regardless of it";
        for words in [
            warning,
            &"⚠️".repeat(60),
            &format!("zsh: {} the end", "❤️".repeat(60)),
        ] {
            let banner = banner_countdown(in_16, Some(words), 100);
            assert!(
                banner.width() <= 100,
                "{} columns: {banner}",
                banner.width()
            );
            assert!(!banner.contains('\u{FE0F}'), "{banner}");
            assert!(banner.ends_with("— reconnecting in 16s — Ctrl-C to exit]"));
        }
    }

    /// **Review 3's Low 2:** a far side's bidirectional controls go with the other controls,
    /// so an override left open cannot reorder pmacs's own words after it, on every surface
    /// its line reaches. At `e111983` they pass.
    #[test]
    fn a_far_sides_bidi_controls_reach_no_terminal() {
        let bidi = [
            '\u{061C}', '\u{200E}', '\u{200F}', '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}',
            '\u{202E}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
        ];
        for c in bidi {
            let line = far_side_line(format!("bash: {c}reversed words\n").as_bytes());
            assert_eq!(line, "bash: reversed words", "U+{:04X}", u32::from(c));
        }
        let said = far_side_line("Permission denied\u{202E} (publickey).\n".as_bytes());
        assert!(!said.contains('\u{202E}'), "{said:?}");
        assert!(!format_stderr_tail_for_display("x\u{2067}y").contains('\u{2067}'));
    }

    /// **Review 3's Low 5:** H1's refusal on a host's ordinary runtime paths keeps its reason
    /// in the countdown at 80 columns: the words keep their first clause, and of it the end.
    /// At `e111983` the start was kept and `is too permissive` cut away.
    #[test]
    fn the_countdown_keeps_a_refusals_reason_at_80_columns() {
        use unicode_width::UnicodeWidthStr as _;
        let in_16 = Duration::from_secs(16);
        for dir in ["/run/user/1001/pmacs", "/tmp/pmacs-1001/pmacs"] {
            let words = format!(
                "pmacs: directory {dir} is too permissive (mode 0o755); expected mode 0700 or \
                 stricter"
            );
            let banner = banner_countdown(in_16, Some(&words), 80);
            assert!(banner.contains("is too permissive"), "{banner}");
            assert!(banner.width() <= 80, "{banner}");
            let wide = banner_countdown(in_16, Some(&words), 200);
            assert!(
                wide.contains(&words),
                "a pane with room reads it whole: {wide}"
            );
        }
        assert_eq!(
            fit_far_side_words(
                "ssh: connect to host farhost port 22: Connection refused",
                30
            ),
            "...port 22: Connection refused"
        );
    }

    /// A far side's words and its status, in place of a frame length; 127's own sentence
    /// is still there, and a refusal reads whole.
    #[test]
    fn far_side_said_reads_its_words_and_its_status() {
        let said = AttachError::FarSideSaid {
            code: Some(127),
            words: "bash: line 1: pmacs: command not found".to_owned(),
        }
        .to_string();
        assert!(
            said.starts_with("the far side said: bash: line 1: pmacs: command not found; "),
            "{said}"
        );
        assert!(said.contains("command not found on remote"), "{said}");
        assert!(said.contains("which pmacs"), "{said}");
        let said = AttachError::FarSideSaid {
            code: Some(1),
            words: "pmacs: directory /x is too permissive".to_owned(),
        }
        .to_string();
        assert_eq!(
            said,
            "the far side said: pmacs: directory /x is too permissive; SSH session exited 1"
        );
    }

    /// The SSH route's stops, one verdict at a time.
    // One assertion a verdict, read in the order the route meets them.
    #[allow(clippy::too_many_lines)]
    #[test]
    fn a_lost_session_decides_the_remote_verdict() {
        let target = AttachTarget::Ssh {
            host: "farhost".to_owned(),
            user: None,
            instance_name: None,
        };
        let lost = |bootstrapped, cut_short| {
            Some(SessionLoss {
                bootstrapped,
                cut_short,
                notice_stood: true,
                exit: SshExit::Clean,
            })
        };
        let reconnect = || ReconnectVerdict::Reconnect {
            error: AttachError::Io(io::Error::other("ssh exited 1")),
        };
        // A session that worked and was lost reconnects, whatever its end: the
        // bridge's own, with a frame cut short or at a frame's boundary...
        for cut_short in [true, false] {
            let mut count = 0;
            assert!(matches!(
                remote_verdict(
                    ReconnectVerdict::ExitClean,
                    lost(true, cut_short),
                    &mut count,
                    &target,
                    None,
                ),
                ReconnectVerdict::Reconnect { .. }
            ));
        }
        // ...and an ssh that failed, on the attempt whose first handshake it was.
        let mut count = 0;
        assert!(matches!(
            remote_verdict(
                ReconnectVerdict::ReconnectHandshake {
                    error: AttachError::Io(io::Error::other("ssh exited 1")),
                },
                lost(true, false),
                &mut count,
                &target,
                None,
            ),
            ReconnectVerdict::Reconnect { .. }
        ));
        // Sessions cut before their first frame go on until the cap, by either end...
        for last in [
            ReconnectVerdict::ExitClean,
            ReconnectVerdict::ReconnectHandshake {
                error: AttachError::Io(io::Error::other("ssh exited 1")),
            },
            reconnect(),
        ] {
            let mut count = 0;
            for _ in 1..HANDSHAKE_RETRY_CAP {
                assert!(matches!(
                    remote_verdict(
                        ReconnectVerdict::ExitClean,
                        lost(false, true),
                        &mut count,
                        &target,
                        None,
                    ),
                    ReconnectVerdict::Reconnect { .. }
                ));
            }
            assert!(matches!(
                remote_verdict(last, lost(false, true), &mut count, &target, None),
                ReconnectVerdict::ExitProtocolError {
                    error: AttachError::SessionsDropped {
                        attempts: HANDSHAKE_RETRY_CAP,
                        cut_short: true,
                        far_side: None,
                    }
                }
            ));
        }
        // ...a first frame starts the count again...
        let mut count = HANDSHAKE_RETRY_CAP - 1;
        assert!(matches!(
            remote_verdict(reconnect(), lost(true, true), &mut count, &target, None),
            ReconnectVerdict::Reconnect { .. }
        ));
        assert_eq!(count, 0);
        // ...and a network that is down reaches no pump and counts nothing.
        let mut count = HANDSHAKE_RETRY_CAP;
        assert!(matches!(
            remote_verdict(reconnect(), None, &mut count, &target, None),
            ReconnectVerdict::Reconnect { .. }
        ));
        // A session that ended on purpose lost nothing, and ends the route.
        let mut count = 0;
        assert!(matches!(
            remote_verdict(ReconnectVerdict::ExitClean, None, &mut count, &target, None),
            ReconnectVerdict::ExitClean
        ));
        // What cannot be retried stops, whatever the session reached.
        let mut count = 0;
        assert!(matches!(
            remote_verdict(
                ReconnectVerdict::ExitProtocolError {
                    error: AttachError::Terminal(io::Error::other("crossterm broke")),
                },
                lost(true, false),
                &mut count,
                &target,
                None,
            ),
            ReconnectVerdict::ExitProtocolError {
                error: AttachError::Terminal(_)
            }
        ));
    }

    /// The older cap's two sentences on the remote route, by how the last session ended,
    /// and the file as the socket's name decides it.
    #[test]
    fn the_older_cap_names_the_far_sides_stderr_by_how_the_last_session_ended() {
        let on = |user: Option<&str>, instance_name: Option<&str>| AttachTarget::Ssh {
            host: "farhost".to_owned(),
            user: user.map(str::to_owned),
            instance_name: instance_name.map(str::to_owned),
        };
        let stopped_by = |cut_short, exit, target: &AttachTarget| {
            let mut count = HANDSHAKE_RETRY_CAP - 1;
            let ReconnectVerdict::ExitProtocolError { error } = remote_verdict(
                ReconnectVerdict::ExitClean,
                Some(SessionLoss {
                    bootstrapped: false,
                    cut_short,
                    notice_stood: true,
                    exit,
                }),
                &mut count,
                target,
                None,
            ) else {
                panic!("it stops at the cap");
            };
            error.to_string()
        };
        // The owner's words, ruled 2026-11-07, but for the host, the instance and the count.
        assert_eq!(
            stopped_by(false, SshExit::Clean, &on(Some("someone"), Some("n"))),
            "the daemon on someone@farhost closed 3 sessions in a row before their first \
             frame, so this client has stopped trying; if pmacs started it, its stderr is in \
             pmacs/n.sock.stderr under that host's $XDG_RUNTIME_DIR, or /tmp/pmacs-<uid> \
             where that is unset"
        );
        assert_eq!(
            stopped_by(false, SshExit::Code(255), &on(Some("someone"), None)),
            "ssh to someone@farhost ended 3 times in a row before a first frame (status 255), \
             so this client has stopped trying"
        );
        assert!(
            stopped_by(false, SshExit::Clean, &on(None, None)).starts_with(
                "the daemon on farhost closed 3 sessions in a row before their first frame"
            ),
            "the user only where the route was given it"
        );
        assert!(
            stopped_by(false, SshExit::Clean, &on(None, None))
                .ends_with(" is in pmacs/default.sock.stderr under that host's $XDG_RUNTIME_DIR, or /tmp/pmacs-<uid> where that is unset"),
        );
        assert!(
            stopped_by(false, SshExit::Clean, &on(None, Some("/srv/d.sock")))
                .ends_with(" is in /srv/d.sock.stderr"),
            "a path is the far side's own"
        );
        let signalled = stopped_by(false, SshExit::Signal, &on(None, None));
        assert!(signalled.contains("(a signal)"), "{signalled}");
        assert!(
            !signalled.contains("stderr") && !signalled.contains("daemon"),
            "no daemon was involved: {signalled}"
        );
        for exit in [SshExit::Clean, SshExit::Code(255)] {
            let cut = stopped_by(true, exit, &on(None, None));
            assert!(cut.starts_with(FRAME_CUT_SHORT_SENTENCE), "{cut}");
            assert!(!cut.contains("stderr"), "the bound's, not a death: {cut}");
        }
    }

    /// The new-daemons cap takes `this client` and the same words for where, its `the last
    /// one's` standing, since its subject is a new daemon.
    #[test]
    fn the_new_daemons_cap_reads_as_ruled() {
        let target = AttachTarget::Ssh {
            host: "farhost".to_owned(),
            user: Some("someone".to_owned()),
            instance_name: Some("n".to_owned()),
        };
        assert_eq!(
            AttachError::NewDaemonsKeepDying {
                stderr: FarSideStderr::of(&target),
                attempts: 3,
            }
            .to_string(),
            "a new daemon on someone@farhost died 3 times in a row, each session lasting under \
             30 s, so this client has stopped trying; the last one's stderr is in \
             pmacs/n.sock.stderr under that host's $XDG_RUNTIME_DIR, or /tmp/pmacs-<uid> \
             where that is unset"
        );
    }

    /// What a `Hello` says of a new daemon is said until a session reaches the first
    /// frame that shows it, and not after, and the stronger of two readings stands.
    #[test]
    fn a_new_daemon_is_said_until_a_first_frame_shows_it() {
        let start = Instant::now();
        let at = |secs| start + Duration::from_secs(secs);
        let plain = Some(Notice::plain(NOTICE_FRESH_VIEW));
        let mut memory = SessionMemory::default();
        memory.heard_at(FrontendId(5), 1_000, at(0));
        assert_eq!(memory.notice(), None, "nothing was lost");
        memory.lost(true, NOTICE_FRESH_VIEW);
        memory.heard_at(FrontendId(6), 1_010, at(10));
        assert_eq!(memory.notice(), plain, "the same daemon");
        memory.lost(true, NOTICE_FRESH_VIEW);
        memory.heard_at(FrontendId(7), 0, at(20));
        assert_eq!(memory.notice(), Some(NOTICE_MAYBE_NEW_DAEMON), "younger");
        // Lost before its first frame: the user has still not been told.
        memory.lost(false, NOTICE_FRESH_VIEW_CUT_SHORT);
        memory.heard_at(FrontendId(8), 1, at(21));
        assert_eq!(memory.notice(), Some(NOTICE_MAYBE_NEW_DAEMON));
        // A number not past the last says more, and goes on being said.
        memory.lost(false, NOTICE_FRESH_VIEW_CUT_SHORT);
        memory.heard_at(FrontendId(2), 0, at(22));
        assert_eq!(memory.notice(), Some(NOTICE_NEW_DAEMON), "certain");
        memory.lost(false, NOTICE_FRESH_VIEW_CUT_SHORT);
        memory.heard_at(FrontendId(3), 1, at(23));
        assert_eq!(memory.notice(), Some(NOTICE_NEW_DAEMON));
        // Told, so a later loss on that daemon says only its own reason.
        memory.lost(true, NOTICE_FRESH_VIEW);
        memory.heard_at(FrontendId(4), 11, at(33));
        assert_eq!(memory.notice(), plain);
    }

    /// The owner's words, each at the width it was ruled for, so a sentence is not cut
    /// before the clause that matters.
    #[test]
    fn the_notice_stands_five_seconds_as_ruled() {
        assert_eq!(NOTICE_FLOOR, Duration::from_secs(5));
    }

    #[test]
    fn a_notice_takes_the_form_that_fits() {
        let first = "[pmacs: new daemon; work not autosaved is gone; M-x recover-file]";
        let second = "[pmacs reconnected: this may be a new daemon; C-x b shows what it holds]";
        let first_cols = u32::try_from(first.len()).expect("cols");
        let second_cols = u32::try_from(second.len()).expect("cols");
        assert_eq!(NOTICE_NEW_DAEMON.fitting(first_cols), first);
        assert_eq!(
            NOTICE_NEW_DAEMON.fitting(first_cols - 1),
            "[new daemon; work not autosaved is gone]"
        );
        assert_eq!(
            NOTICE_NEW_DAEMON.fitting(40).len(),
            40,
            "ruled at 40 columns"
        );
        assert_eq!(NOTICE_MAYBE_NEW_DAEMON.fitting(second_cols), second);
        assert_eq!(
            NOTICE_MAYBE_NEW_DAEMON.fitting(second_cols - 1),
            "[this may be a new daemon]"
        );
        assert_eq!(
            Notice::plain(NOTICE_FRESH_VIEW).fitting(10),
            NOTICE_FRESH_VIEW,
            "one form, which the row then cuts"
        );
    }

    /// What the next session says of a lost one.
    #[test]
    fn a_lost_session_says_why_on_the_next_one() {
        let lost = |cut_short| SessionLoss {
            bootstrapped: true,
            cut_short,
            notice_stood: true,
            exit: SshExit::Clean,
        };
        let failed = ReconnectVerdict::Reconnect {
            error: AttachError::Io(io::Error::other("ssh exited 255")),
        };
        assert_eq!(
            lost(false).notice(&ReconnectVerdict::ExitClean),
            NOTICE_FRESH_VIEW
        );
        assert_eq!(lost(false).notice(&failed), NOTICE_FRESH_VIEW_LINK);
        for verdict in [ReconnectVerdict::ExitClean, failed] {
            assert_eq!(lost(true).notice(&verdict), NOTICE_FRESH_VIEW_CUT_SHORT);
        }
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
        assert_eq!(
            banner_remote_reconnect(),
            "[pmacs reconnecting... Ctrl-C to exit]"
        );
    }

    #[test]
    fn a_reconnects_hello_deadline_is_120_s_as_ruled() {
        assert_eq!(RECONNECT_HELLO_DEADLINE, Duration::from_mins(2));
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
