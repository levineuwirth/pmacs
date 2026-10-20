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
//!   v27 cannot be stopped this way: it refuses the attach with
//!   `VersionMismatch`, which names its ceiling, and that is reported
//!   as an older daemon to stop with a signal. The client cannot know
//!   sooner, since `Hello` carries the advertised baseline alone and
//!   never the daemon's ceiling (review 1's Low 1: the first cut
//!   compared the version the `Hello` implied, which is this build's
//!   own offer for every daemon since v20, and the arm never fired).
//!
//! Both clean up a stale socket: a socket file nothing listens on
//! (`ECONNREFUSED` on connect) is the leftover of a daemon that died
//! without its `cleanup`, and is removed so a later `pmacs --gpu` or
//! `--daemon` finds the path free. A live daemon is never touched this
//! way --- it holds the lockfile and answers its socket --- and a path
//! that is not a socket is left alone and reported. The verdict is read
//! from the path, not the errno (`pmacs_protocol::socket_file`): Linux
//! refuses a connect to a regular file with `ECONNREFUSED`, macOS with
//! `ENOTSOCK`, and the first cut's split on the former printed the raw
//! errno on macOS where this sentence was promised (review 1's High 1).

use std::io::ErrorKind;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use pmacs_protocol::{ConnectFailure, SESSION_MIN_VERSION, classify_connect_failure};

use crate::protocol::{
    AttachRequest, FrontendCapabilities, FrontendEvent, GoodbyeReason, Hello, InstanceMessage,
    PROTOCOL_VERSION, requested_protocol_version,
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
    /// The daemon refused the attach the stop request rides on. A
    /// `VersionMismatch` is the one way an older daemon says it has no
    /// stop request, and is reported as such.
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
            Self::Rejected(GoodbyeReason::VersionMismatch { server, client })
                if server < &SESSION_MIN_VERSION =>
            {
                write!(
                    f,
                    "the daemon speaks protocol v{server}, before the stop request \
                     (v{SESSION_MIN_VERSION}); stop it with a signal"
                )
            }
            Self::Rejected(GoodbyeReason::VersionMismatch { server, client }) => write!(
                f,
                "the daemon refused protocol v{client}, the version this build offered \
                 (it speaks v{server}); a newer pmacs is needed to stop it this way"
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
        // The daemon's own ceiling is not in its `Hello`, which carries
        // the baseline every shipped frontend accepts; what this build
        // would offer it is the one version this line can name.
        writeln!(
            f,
            "  protocol baseline v{} (this build offers v{PROTOCOL_VERSION}; the daemon's \
             ceiling is not in its Hello)",
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
        Err(e) => Err(connect_failure(socket, e)),
    }
}

/// E9.4 --- what a failed connect to `socket` means, by the path: a
/// socket file nothing answers is removed (a live daemon never refuses
/// its own socket, and one that has not yet bound holds the path
/// free), a path that is not a socket is left alone and named, and
/// nothing at the path is no daemon. Any other failure is reported as
/// the error it was.
pub fn connect_failure(socket: &Path, error: std::io::Error) -> DaemonControlError {
    match classify_connect_failure(socket, &error) {
        ConnectFailure::Missing => DaemonControlError::NoDaemon(socket.to_path_buf()),
        ConnectFailure::Stale => match std::fs::remove_file(socket) {
            Ok(()) => DaemonControlError::StaleSocketRemoved(socket.to_path_buf()),
            Err(e) => DaemonControlError::StaleSocket(socket.to_path_buf(), e),
        },
        ConnectFailure::NotASocket => DaemonControlError::NotASocket(socket.to_path_buf()),
        ConnectFailure::Other => DaemonControlError::Io(error),
        ConnectFailure::Inspect(e) => DaemonControlError::Io(e),
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
    // The offer is this build's own version: `Hello` carries only the
    // baseline, so whether the daemon speaks v27 is learned from its
    // answer to the `AttachRequest`, a `VersionMismatch` if not.
    let offer = requested_protocol_version(hello.protocol_version);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// macOS's `connect(2)` on a regular file: `ENOTSOCK`, which `std`
    /// carries under no named kind. Linux returns `ECONNREFUSED` there,
    /// so the E9.4 row on a Linux leg never reaches this shape; this
    /// row does, by synthesizing the error the macOS legs of CI printed
    /// raw at `e7c8fd5`.
    fn enotsock() -> std::io::Error {
        std::io::Error::other("Socket operation on non-socket (os error 38)")
    }

    #[test]
    fn a_non_socket_is_named_and_left_alone_on_an_enotsock_connect() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let plain = dir.path().join("plain.sock");
        std::fs::write(&plain, "not a socket").expect("write");
        let verdict = connect_failure(&plain, enotsock());
        assert!(
            matches!(&verdict, DaemonControlError::NotASocket(p) if p == &plain),
            "{verdict:?}"
        );
        assert_eq!(
            verdict.to_string(),
            format!("{} exists and is not a socket; left alone", plain.display())
        );
        assert!(plain.exists());
        // Linux's answer for the same file: the same verdict.
        let refused = std::io::Error::new(ErrorKind::ConnectionRefused, "Connection refused");
        assert!(matches!(
            connect_failure(&plain, refused),
            DaemonControlError::NotASocket(_)
        ));
        assert!(plain.exists());
    }

    /// An older daemon refuses the v27 attach with `VersionMismatch`
    /// naming its ceiling; that is the one way the stop client learns
    /// it, and it says so (review 1's Low 1: the `TooOldToStop` arm it
    /// replaces compared a version the `Hello` cannot carry).
    #[test]
    fn a_version_mismatch_names_an_older_daemon_to_stop_with_a_signal() {
        let older = DaemonControlError::Rejected(GoodbyeReason::VersionMismatch {
            server: SESSION_MIN_VERSION - 1,
            client: PROTOCOL_VERSION,
        });
        let text = older.to_string();
        assert!(
            text.contains(&format!("speaks protocol v{}", SESSION_MIN_VERSION - 1))
                && text.contains("stop it with a signal"),
            "{text}"
        );
        let newer = DaemonControlError::Rejected(GoodbyeReason::VersionMismatch {
            server: PROTOCOL_VERSION + 5,
            client: PROTOCOL_VERSION,
        });
        let text = newer.to_string();
        assert!(
            text.contains("a newer pmacs is needed") && !text.contains("signal"),
            "{text}"
        );
    }

    #[test]
    fn a_refused_socket_file_is_removed_and_a_missing_path_is_no_daemon() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let stale = dir.path().join("stale.sock");
        drop(std::os::unix::net::UnixListener::bind(&stale).expect("bind"));
        let refused = std::io::Error::new(ErrorKind::ConnectionRefused, "Connection refused");
        assert!(matches!(
            connect_failure(&stale, refused),
            DaemonControlError::StaleSocketRemoved(_)
        ));
        assert!(!stale.exists(), "the stale socket file is gone");
        assert!(matches!(
            connect_failure(&stale, enotsock()),
            DaemonControlError::NoDaemon(_)
        ));
    }
}
