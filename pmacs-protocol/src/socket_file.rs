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

/// Where a daemon a frontend or a bridge starts writes its stderr: beside the socket,
/// so it lives in the runtime directory the socket already proved writable and dies
/// with it. Before E2.4 the window's stream went to `/dev/null`, and before E10r's fix
/// round 2 the bridge's did, so a daemon that died said nothing to anyone. It holds every
/// daemon started there, each under a line saying when and by whom
/// ([`open_daemon_stderr`]), so what is in it can be dated by whoever reads it.
#[must_use]
pub fn daemon_stderr_path(socket_path: &Path) -> std::path::PathBuf {
    let mut name = socket_path.as_os_str().to_os_string();
    name.push(".stderr");
    std::path::PathBuf::from(name)
}

/// How large [`daemon_stderr_path`] grows before the next start moves it aside.
pub const DAEMON_STDERR_CAP: u64 = 1024 * 1024;

/// Open [`daemon_stderr_path`] for a daemon `who` is about to start, on the bridge's route
/// and the window's: mode 0600, which also tightens a file an earlier version left wider,
/// and following no link at the file itself, since a link planted there would otherwise be
/// written through. `O_NOFOLLOW` reads the last component alone, so a link at a directory
/// above it, `pmacs/` say, is followed: under `$XDG_RUNTIME_DIR` only the user can make
/// one, and under `/tmp/pmacs-<uid>`, where another local user can be first, that is
/// #481's and #488's (E10r's review 3). A caller falls back to the null device on an
/// error: the daemon matters more than its log.
///
/// It is appended to, under a line saying when and by whom, so that one crash's words
/// outlast the reconnect that starts the next daemon, and two starting together keep every
/// line (the owner's ruling, E10r review 2: a rename at each start could still lose a
/// crash's text to two TUIs reconnecting together). Past [`DAEMON_STDERR_CAP`] the file moves
/// to `.stderr.1`, over the one before. Two starts at that moment may both move, and the
/// second then moves the first's new file over the old one, so the old one's text is lost.
pub fn open_daemon_stderr(socket_path: &Path, who: &str) -> io::Result<std::fs::File> {
    use io::Write as _;
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
    let path = daemon_stderr_path(socket_path);
    if std::fs::symlink_metadata(&path)
        .is_ok_and(|meta| meta.is_file() && meta.len() > DAEMON_STDERR_CAP)
    {
        let mut older = path.clone().into_os_string();
        older.push(".1");
        let _ = std::fs::rename(&path, older);
    }
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    let line = format!(
        "--- {}: a daemon started by {who}, pid {} ---\n",
        utc_now(),
        std::process::id()
    );
    file.write_all(line.as_bytes())?;
    Ok(file)
}

/// Now, as `2026-11-07T21:04:05Z`.
fn utc_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    utc_at(secs)
}

/// `secs` past the epoch, as `2026-11-07T21:04:05Z`. No date crate here, and this is the one
/// date this crate writes: Howard Hinnant's `civil_from_days`, over the positive years.
fn utc_at(secs: u64) -> String {
    let days = secs / 86_400;
    let rest = secs % 86_400;
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3_600,
        rest / 60 % 60,
        rest % 60
    )
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
    fn the_stderr_file_is_private_and_a_link_there_is_not_followed() {
        use std::os::unix::fs::PermissionsExt as _;
        let scratch = Scratch::new("stderr");
        let socket = scratch.0.join("d.sock");
        let path = daemon_stderr_path(&socket);
        drop(open_daemon_stderr(&socket, "a row").expect("a fresh file"));
        let mode = |p: &Path| std::fs::metadata(p).expect("stat").permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);

        // A file an earlier version left readable is tightened.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");
        drop(open_daemon_stderr(&socket, "a row").expect("the file again"));
        assert_eq!(mode(&path), 0o600);

        std::fs::remove_file(&path).expect("remove it");
        let victim = scratch.0.join("victim");
        std::fs::write(&victim, "the victim's own\n").expect("the victim");
        std::os::unix::fs::symlink(&victim, &path).expect("the link");
        assert!(
            open_daemon_stderr(&socket, "a row").is_err(),
            "it followed the link"
        );
        assert_eq!(
            std::fs::read_to_string(&victim).expect("the victim"),
            "the victim's own\n"
        );
    }

    /// H9: what is there stays, under a line at each start saying when and by whom.
    #[test]
    fn the_stderr_file_keeps_what_it_held_under_a_line_at_each_start() {
        use io::Write as _;
        let scratch = Scratch::new("appended");
        let socket = scratch.0.join("d.sock");
        let path = daemon_stderr_path(&socket);
        let mut first = open_daemon_stderr(&socket, "the first").expect("a fresh file");
        first.write_all(b"THE CRASH'S WORDS\n").expect("write");
        let mut second = open_daemon_stderr(&socket, "the second").expect("the file again");
        // Each appends where the file ends, the one opened first included.
        second.write_all(b"THE NEXT ONE'S\n").expect("write");
        first.write_all(b"THE CRASH'S LAST\n").expect("write");
        let held = std::fs::read_to_string(&path).expect("the file");
        let lines: Vec<&str> = held.lines().collect();
        assert_eq!(lines.len(), 5, "{held}");
        assert!(
            lines[0].contains("a daemon started by the first, pid "),
            "{held}"
        );
        assert_eq!(lines[1], "THE CRASH'S WORDS", "{held}");
        assert!(
            lines[2].contains("a daemon started by the second, pid "),
            "{held}"
        );
        assert_eq!(lines[3], "THE NEXT ONE'S", "{held}");
        assert_eq!(lines[4], "THE CRASH'S LAST", "{held}");
    }

    /// H9: past the cap the file moves aside, over the one before, and it does not until
    /// then. A link at that path is left alone, by the rename as by the open.
    #[test]
    fn the_stderr_file_moves_aside_past_its_cap() {
        let scratch = Scratch::new("rotated");
        let socket = scratch.0.join("d.sock");
        let path = daemon_stderr_path(&socket);
        let older = scratch.0.join("d.sock.stderr.1");
        let cap = usize::try_from(DAEMON_STDERR_CAP).expect("cap");

        std::fs::write(&path, vec![b'a'; cap]).expect("at the cap");
        drop(open_daemon_stderr(&socket, "a row").expect("open"));
        assert!(!older.exists(), "at the cap it stays");
        assert!(std::fs::metadata(&path).expect("stat").len() > DAEMON_STDERR_CAP);

        drop(open_daemon_stderr(&socket, "a row").expect("open"));
        assert_eq!(
            std::fs::read(&older).expect("moved").first().copied(),
            Some(b'a'),
            "past it, it moves"
        );
        let fresh = std::fs::read_to_string(&path).expect("the new file");
        assert_eq!(fresh.lines().count(), 1, "{fresh}");

        std::fs::write(&path, vec![b'b'; cap + 1]).expect("past it again");
        drop(open_daemon_stderr(&socket, "a row").expect("open"));
        assert_eq!(
            std::fs::read(&older).expect("moved").first().copied(),
            Some(b'b'),
            "over the one before"
        );

        std::fs::remove_file(&path).expect("remove it");
        let target = scratch.0.join("target");
        std::fs::write(&target, vec![b'c'; cap + 1]).expect("the target");
        std::os::unix::fs::symlink(&target, &path).expect("the link");
        assert!(open_daemon_stderr(&socket, "a row").is_err());
        assert_eq!(
            std::fs::read(&older).expect("moved").first().copied(),
            Some(b'b'),
            "a link is not moved"
        );
    }

    #[test]
    fn the_line_says_a_real_date() {
        assert_eq!(utc_at(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc_at(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(utc_at(1_793_999_045), "2026-11-06T21:04:05Z");
        assert_eq!(utc_at(4_107_542_399), "2100-02-28T23:59:59Z");
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
