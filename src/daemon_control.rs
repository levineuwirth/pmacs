// daemon_control.rs --- `pmacs --daemon --status` and `--stop` over the socket (E9.4).

//! The two lifecycle questions a user asks a daemon from a shell,
//! answered over its socket rather than by `pgrep` and `kill`:
//!
//! * [`status`] connects, reads the server-first [`Hello`] and reports
//!   what it carries --- the daemon's version, protocol baseline,
//!   instance name, uptime and working directory --- then closes
//!   without attaching. No wire change: every daemon since v6 answers
//!   this way, and a `Hello` read is what every readiness wait does.
//! * [`stop`] attaches as a v27 grid frontend and sends
//!   [`FrontendEvent::Shutdown`]; the daemon answers every attached
//!   frontend with `Goodbye(ShuttingDown)` and exits. A daemon below
//!   v27 cannot be stopped this way and is named as such.
//!
//! Both clean up a stale socket: a socket file nothing listens on
//! (`ECONNREFUSED` on connect) is the leftover of a daemon that died
//! without its `cleanup`, and is removed so a later `pmacs --gpu` or
//! `--daemon` finds the path free. A live daemon is never touched this
//! way --- it holds the lockfile and answers its socket --- and a path
//! that is not a socket is left alone and reported.

use std::io::ErrorKind;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use pmacs_protocol::SESSION_MIN_VERSION;

use crate::protocol::{
    AttachRequest, FrontendCapabilities, FrontendEvent, GoodbyeReason, Hello, InstanceMessage,
    PROTOCOL_VERSION, negotiated_session_version, requested_protocol_version,
};
use crate::transport::{read_message, write_message};

/// How long either question waits on the daemon's answer.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(5);

/// What `--status` or `--stop` found at the socket.
#[derive(Debug)]
pub enum DaemonControlError {
    /// Nothing at the path: no daemon has run there, or it cleaned up.
    NoDaemon(PathBuf),
    /// A socket file nothing answers, now removed.
    StaleSocketRemoved(PathBuf),
    /// A socket file nothing answers, which could not be removed.
    StaleSocket(PathBuf, std::io::Error),
    /// The path exists and is not a socket; left alone.
    NotASocket(PathBuf),
    /// The daemon answered, but speaks a wire without the stop request.
    TooOldToStop {
        /// The socket that answered.
        socket: PathBuf,
        /// The version the session with it would speak.
        version: u32,
    },
    /// The daemon refused the attach the stop request rides on.
    Rejected(GoodbyeReason),
    /// Any other failure to reach or read the daemon.
    Io(std::io::Error),
}

impl std::fmt::Display for DaemonControlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoDaemon(path) => write!(f, "no daemon at {}", path.display()),
            Self::StaleSocketRemoved(path) => write!(
                f,
                "no daemon at {} (a stale socket nothing answered; removed)",
                path.display()
            ),
            Self::StaleSocket(path, e) => write!(
                f,
                "no daemon at {} (a stale socket nothing answers; removing it failed: {e})",
                path.display()
            ),
            Self::NotASocket(path) => {
                write!(
                    f,
                    "{} exists and is not a socket; left alone",
                    path.display()
                )
            }
            Self::TooOldToStop { socket, version } => write!(
                f,
                "the daemon at {} speaks protocol v{version}, before the stop request (v{}); \
                 stop it with a signal",
                socket.display(),
                SESSION_MIN_VERSION
            ),
            Self::Rejected(reason) => write!(
                f,
                "the daemon refused the stop request's attach: {reason:?}"
            ),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for DaemonControlError {}

impl From<std::io::Error> for DaemonControlError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<crate::transport::TransportError> for DaemonControlError {
    fn from(e: crate::transport::TransportError) -> Self {
        Self::Io(std::io::Error::other(e.to_string()))
    }
}

/// What [`status`] reports: the daemon's `Hello`, read without attaching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonStatus {
    /// The socket answered.
    pub socket: PathBuf,
    /// The daemon's `Hello`.
    pub hello: Hello,
}

impl std::fmt::Display for DaemonStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let id = &self.hello.instance_identity;
        writeln!(f, "daemon at {}", self.socket.display())?;
        writeln!(f, "  pmacs {}", id.pmacs_version)?;
        writeln!(
            f,
            "  protocol baseline v{} (this build speaks v{PROTOCOL_VERSION})",
            self.hello.protocol_version
        )?;
        writeln!(
            f,
            "  instance {}",
            id.instance_name.as_deref().unwrap_or("default")
        )?;
        writeln!(f, "  up {}", describe_uptime(id.uptime_secs))?;
        write!(f, "  cwd {}", id.working_directory)
    }
}

fn describe_uptime(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{h}h {m}m {s}s")
    } else if m > 0 {
        format!("{m}m {s}s")
    } else {
        format!("{s}s")
    }
}

/// Connect to `socket`, cleaning up a stale socket file on the way.
fn connect(socket: &Path) -> Result<UnixStream, DaemonControlError> {
    match UnixStream::connect(socket) {
        Ok(stream) => {
            stream.set_read_timeout(Some(ANSWER_TIMEOUT))?;
            stream.set_write_timeout(Some(ANSWER_TIMEOUT))?;
            Ok(stream)
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {
            Err(DaemonControlError::NoDaemon(socket.to_path_buf()))
        }
        Err(e) if e.kind() == ErrorKind::ConnectionRefused => Err(clear_stale_socket(socket)),
        Err(e) => Err(DaemonControlError::Io(e)),
    }
}

/// E9.4 --- a socket file nothing answers is removed. Called only after
/// a connect was refused, which a live daemon never does: it answers
/// its own socket, and a daemon that has not yet bound holds the path
/// free.
pub fn clear_stale_socket(socket: &Path) -> DaemonControlError {
    use std::os::unix::fs::FileTypeExt as _;
    match std::fs::symlink_metadata(socket) {
        Ok(meta) if meta.file_type().is_socket() => match std::fs::remove_file(socket) {
            Ok(()) => DaemonControlError::StaleSocketRemoved(socket.to_path_buf()),
            Err(e) => DaemonControlError::StaleSocket(socket.to_path_buf(), e),
        },
        Ok(_) => DaemonControlError::NotASocket(socket.to_path_buf()),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            DaemonControlError::NoDaemon(socket.to_path_buf())
        }
        Err(e) => DaemonControlError::Io(e),
    }
}

/// `pmacs --daemon --status`: the daemon's `Hello`, read and reported.
pub fn status(socket: &Path) -> Result<DaemonStatus, DaemonControlError> {
    let mut stream = connect(socket)?;
    let hello: Hello = read_message(&mut stream)?;
    Ok(DaemonStatus {
        socket: socket.to_path_buf(),
        hello,
    })
}

/// `pmacs --daemon --stop`: attach, ask, and wait for the daemon's
/// `Goodbye(ShuttingDown)` or its close. Returns the `Hello` of the
/// daemon that was stopped.
pub fn stop(socket: &Path) -> Result<Hello, DaemonControlError> {
    let mut stream = connect(socket)?;
    let hello: Hello = read_message(&mut stream)?;
    let offer = requested_protocol_version(hello.protocol_version);
    let version = negotiated_session_version(offer);
    if version < SESSION_MIN_VERSION {
        return Err(DaemonControlError::TooOldToStop {
            socket: socket.to_path_buf(),
            version,
        });
    }
    // A grid attach (no semantic bootstrap, no `Resume` step), multi
    // so the single non-multi slot a TUI may hold does not refuse it.
    write_message(
        &mut stream,
        &AttachRequest {
            protocol_version: offer,
            frontend_capabilities: FrontendCapabilities {
                terminal_kind: Some("pmacs --daemon --stop".to_owned()),
                multi_frontend: true,
                ..FrontendCapabilities::default()
            },
            initial_size: pmacs_protocol::CellSize::new(24, 80),
        },
    )?;
    write_message(
        &mut stream,
        &FrontendEvent::Shutdown {
            frontend_id: hello.assigned_frontend_id,
        },
    )?;
    // The answer: a `Goodbye(ShuttingDown)`, or the close that follows
    // it; a rejection of the attach itself is reported as such. The
    // frames the daemon writes to its new grid session meanwhile are
    // read past, under one deadline for the whole answer rather than
    // one per read, so a daemon that keeps painting but never stops is
    // reported, not waited on.
    let deadline = std::time::Instant::now() + ANSWER_TIMEOUT;
    loop {
        if std::time::Instant::now() >= deadline {
            return Err(DaemonControlError::Io(std::io::Error::new(
                ErrorKind::TimedOut,
                format!(
                    "the daemon at {} did not stop within {} s of the request",
                    socket.display(),
                    ANSWER_TIMEOUT.as_secs()
                ),
            )));
        }
        match read_message::<InstanceMessage>(&mut stream) {
            Ok(InstanceMessage::Goodbye(GoodbyeReason::ShuttingDown))
            | Err(crate::transport::TransportError::Eof) => return Ok(hello),
            Ok(InstanceMessage::Goodbye(reason)) => {
                return Err(DaemonControlError::Rejected(reason));
            }
            Ok(_) => {}
            Err(e) => return Err(e.into()),
        }
    }
}
