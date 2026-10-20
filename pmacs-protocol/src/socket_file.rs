//! What a failed `connect(2)` on a Unix socket path means, read from
//! the path rather than from the errno (E9.4, fix round 1).
//!
//! Two callers meet a connect that fails and have to decide whether
//! the path is a daemon's leftover (a socket file nothing answers,
//! safe to remove or to start a daemon over), something that is not a
//! socket at all (left alone, named), or nothing (no daemon has run
//! there): `pmacs --daemon --status` and `--stop` in the `pmacs` crate,
//! and `pmacs-gpu`'s managed attach, which starts a daemon for itself.
//! It lives here because `pmacs-gpu` depends on this crate and never on
//! `pmacs`, as the reconnect schedule does; nothing here is a wire byte.
//!
//! The errno alone does not say. Linux returns `ECONNREFUSED` both for
//! a socket file nothing listens on and for a path that is a regular
//! file; macOS checks the vnode's type before it looks for a listener
//! and returns `ENOTSOCK` for anything that is not a socket, which
//! `std` maps to no named [`io::ErrorKind`]. E9's first cut split on
//! `ConnectionRefused` alone, and its own stale-socket row printed the
//! raw errno on both macOS legs of CI where a sentence was promised. So
//! the split is by the path: on any error but `NotFound` the path is
//! examined, and what it is decides the verdict.

use std::io;
use std::path::Path;

/// What a failed connect to a socket path means.
#[derive(Debug)]
pub enum ConnectFailure {
    /// Nothing is at the path: the connect, or the look at the path
    /// after it, found nothing there.
    Missing,
    /// A socket file nothing answers: the connect was refused and the
    /// path is a socket. A daemon that died without its cleanup leaves
    /// this; a live daemon answers its socket.
    Stale,
    /// The path exists and is not a socket (a regular file, a
    /// directory, a symlink, anything else).
    NotASocket,
    /// A socket file whose connect failed for some other reason; the
    /// caller's own error says what.
    Other,
    /// The path could not be examined: `symlink_metadata` failed with
    /// this error, which is not `NotFound`.
    Inspect(io::Error),
}

/// Classify `error`, returned by a connect to `path`, by what is at
/// `path`. A symlink is examined as itself, not followed: a link at a
/// socket path is left alone by every caller.
#[must_use]
pub fn classify_connect_failure(path: &Path, error: &io::Error) -> ConnectFailure {
    use std::os::unix::fs::FileTypeExt as _;
    if error.kind() == io::ErrorKind::NotFound {
        return ConnectFailure::Missing;
    }
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_socket() => {
            if error.kind() == io::ErrorKind::ConnectionRefused {
                ConnectFailure::Stale
            } else {
                ConnectFailure::Other
            }
        }
        Ok(_) => ConnectFailure::NotASocket,
        Err(e) if e.kind() == io::ErrorKind::NotFound => ConnectFailure::Missing,
        Err(e) => ConnectFailure::Inspect(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;

    /// A directory of this test's own under the ambient temporary
    /// root, removed when dropped. No `tempfile` here: this crate has
    /// no dev-dependencies and the two paths below need none.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("pmacs-socket-file-{tag}-{}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("create the scratch directory");
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// macOS's answer for a connect to a regular file, as `std` carries
    /// it: an errno with no named kind.
    fn enotsock() -> io::Error {
        io::Error::other("Socket operation on non-socket (os error 38)")
    }

    fn refused() -> io::Error {
        io::Error::new(io::ErrorKind::ConnectionRefused, "Connection refused")
    }

    #[test]
    fn a_regular_file_is_not_a_socket_whatever_the_errno() {
        let scratch = Scratch::new("plain");
        let plain = scratch.0.join("plain.sock");
        std::fs::write(&plain, "not a socket").expect("write");
        assert!(matches!(
            classify_connect_failure(&plain, &enotsock()),
            ConnectFailure::NotASocket
        ));
        assert!(matches!(
            classify_connect_failure(&plain, &refused()),
            ConnectFailure::NotASocket
        ));
        assert!(plain.exists(), "classifying removes nothing");
    }

    #[test]
    fn a_refused_socket_file_is_stale_and_another_error_on_it_is_other() {
        let scratch = Scratch::new("stale");
        let stale = scratch.0.join("stale.sock");
        drop(UnixListener::bind(&stale).expect("bind"));
        assert!(matches!(
            classify_connect_failure(&stale, &refused()),
            ConnectFailure::Stale
        ));
        assert!(matches!(
            classify_connect_failure(
                &stale,
                &io::Error::new(io::ErrorKind::PermissionDenied, "denied")
            ),
            ConnectFailure::Other
        ));
        assert!(stale.exists(), "classifying removes nothing");
    }

    #[test]
    fn a_missing_path_is_missing_from_the_connect_or_from_the_look() {
        let scratch = Scratch::new("missing");
        let absent = scratch.0.join("absent.sock");
        assert!(matches!(
            classify_connect_failure(
                &absent,
                &io::Error::new(io::ErrorKind::NotFound, "No such file")
            ),
            ConnectFailure::Missing
        ));
        assert!(matches!(
            classify_connect_failure(&absent, &refused()),
            ConnectFailure::Missing
        ));
    }

    #[test]
    fn a_symlink_is_examined_as_itself() {
        let scratch = Scratch::new("link");
        let target = scratch.0.join("target.sock");
        drop(UnixListener::bind(&target).expect("bind"));
        let link = scratch.0.join("link.sock");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");
        assert!(matches!(
            classify_connect_failure(&link, &refused()),
            ConnectFailure::NotASocket
        ));
    }
}
