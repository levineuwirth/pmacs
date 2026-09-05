// lockfile.rs --- Single-instance daemon lock via flock(2).

//! Single-instance lockfile lifecycle for the M5.5 daemon (T M5.5d).
//!
//! # Why a sibling lockfile, not the socket
//!
//! `bind(2)` on a Unix socket fails with `EADDRINUSE` if the socket
//! file already exists, even when no daemon is holding it (e.g. after
//! a crashed daemon left a stale socket on disk). We could `unlink`
//! before `bind`, but doing so without a lock invites a race: two
//! daemons starting in lockstep, both unlink, both bind, only one
//! wins, the loser sees its bind succeed but its socket is the one
//! the winner already replaced.
//!
//! `flock(2)` on a sibling lockfile (`<socket>.lock`) gives us
//! atomicity: only one daemon holds the exclusive flock at a time, so
//! the unlink-then-bind sequence happens under serialization. The
//! kernel releases the flock automatically when the file descriptor
//! closes, including during a process crash, so a dead daemon's lock
//! never blocks recovery.
//!
//! # Lifecycle
//!
//! 1. [`acquire_lock`] opens or creates `<socket>.lock` mode 0600,
//!    `flock(LOCK_EX | LOCK_NB)`, writes our pid (advisory) to the
//!    file, and returns a [`LockHandle`].
//! 2. The daemon does its unlink-stale-socket and bind work while
//!    holding the [`LockHandle`].
//! 3. On clean shutdown the daemon calls [`LockHandle::release`],
//!    which unlinks the lockfile while still holding the lock, then
//!    drops it (releasing the flock).
//! 4. On crash, the kernel releases the flock; the lockfile remains
//!    on disk and gets reused (with an updated pid) by the next
//!    daemon.
//!
//! # Pid is advisory
//!
//! The pid in the lockfile is for diagnostic display only ("daemon
//! already running at `<path>` (pid 12345)"). We never make a decision
//! based on it — the `flock` is the source of truth. A stale pid that
//! happens to match a live unrelated process is harmless: we just
//! print a slightly misleading error.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use nix::errno::Errno;
use nix::fcntl::{Flock, FlockArg};

/// Errors from acquiring or releasing a daemon lockfile.
#[derive(Debug)]
pub enum LockError {
    /// Another process holds the lock. The advisory pid is the value
    /// the holder wrote to the file (best-effort; may be `None` if
    /// the file was empty or unreadable, or stale if the holder
    /// hadn't written its pid yet).
    AlreadyHeld {
        /// Pid the holder wrote into the lockfile, or `None` if no
        /// readable pid was present.
        advisory_pid: Option<u32>,
    },
    /// I/O error opening, locking, or writing the lockfile.
    Io(std::io::Error),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyHeld {
                advisory_pid: Some(pid),
            } => {
                write!(f, "daemon already running (pid {pid})")
            }
            Self::AlreadyHeld { advisory_pid: None } => {
                write!(f, "daemon already running (pid unknown)")
            }
            Self::Io(e) => write!(f, "lockfile I/O error: {e}"),
        }
    }
}

impl std::error::Error for LockError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::AlreadyHeld { .. } => None,
        }
    }
}

impl From<std::io::Error> for LockError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Sibling-lockfile path for a given socket path: `<socket>.lock`.
#[must_use]
pub fn lockfile_path_for(socket_path: &Path) -> PathBuf {
    let mut s = socket_path.as_os_str().to_os_string();
    s.push(".lock");
    PathBuf::from(s)
}

/// Holder for the exclusive flock on a daemon lockfile.
///
/// The wrapped `Flock<File>` releases the kernel-level lock when
/// dropped (including during a panic or crash). Use [`Self::release`]
/// for the clean-shutdown path which also unlinks the lockfile.
#[derive(Debug)]
pub struct LockHandle {
    // Held only for its Drop semantics: the kernel releases the
    // flock when the file descriptor closes. Underscored so dead-code
    // lints understand the field's purpose.
    _flock: Flock<File>,
    lockfile_path: PathBuf,
}

impl LockHandle {
    /// The lockfile this handle owns.
    #[must_use]
    pub fn lockfile_path(&self) -> &Path {
        &self.lockfile_path
    }

    /// Clean-shutdown release: unlink the lockfile while we still
    /// hold the lock, then drop the handle (releasing the flock).
    ///
    /// Unlinking before lock release is deliberate: a racing acquirer
    /// who starts after the unlink but before our drop will simply
    /// re-create the lockfile on next [`acquire_lock`], proceeding
    /// once our drop releases the flock. There is no window in which
    /// they could acquire a lock the kernel doesn't recognize as
    /// ours.
    pub fn release(self) -> std::io::Result<()> {
        fs::remove_file(&self.lockfile_path)
        // self drops here, releasing the flock as the fd closes.
    }
}

/// Acquire the exclusive flock on `<socket_path>.lock`.
///
/// Creates the lockfile mode 0600 if it doesn't exist. On success,
/// truncates the file and writes our pid (advisory). On
/// `EWOULDBLOCK`, reads the holder's advisory pid (best-effort) and
/// returns [`LockError::AlreadyHeld`].
pub fn acquire_lock(socket_path: &Path) -> Result<LockHandle, LockError> {
    let lockfile_path = lockfile_path_for(socket_path);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&lockfile_path)?;

    let mut flock = match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
        Ok(f) => f,
        Err((f, Errno::EWOULDBLOCK)) => {
            let advisory_pid = read_advisory_pid(&f);
            return Err(LockError::AlreadyHeld { advisory_pid });
        }
        Err((_f, errno)) => {
            return Err(LockError::Io(std::io::Error::from_raw_os_error(
                errno as i32,
            )));
        }
    };

    // We hold the lock. Refresh the advisory pid: truncate then
    // write. Failures here are unusual (we just opened the file
    // ourselves) but we propagate them so a half-set-up lockfile
    // doesn't get committed.
    flock.set_len(0)?;
    flock.seek(SeekFrom::Start(0))?;
    let pid = std::process::id();
    flock.write_all(format!("{pid}\n").as_bytes())?;

    Ok(LockHandle {
        _flock: flock,
        lockfile_path,
    })
}

fn read_advisory_pid(file: &File) -> Option<u32> {
    // Caller has &File but the file's seek cursor may be anywhere.
    // Re-open the path would race with the holder's truncate; use
    // `read_at` semantics by cloning the fd and seeking the clone.
    // Simpler still: borrow the file, save+restore cursor. But the
    // caller passed us &File; we can't seek without &mut. Use
    // `try_clone` to get an owned File we can manipulate freely.
    let mut clone = file.try_clone().ok()?;
    clone.seek(SeekFrom::Start(0)).ok()?;
    let mut buf = String::new();
    clone.read_to_string(&mut buf).ok()?;
    buf.trim().parse::<u32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;
    use tempfile::TempDir;

    #[test]
    fn lockfile_path_appends_dot_lock() {
        let socket = Path::new("/run/pmacs/default.sock");
        let lock = lockfile_path_for(socket);
        assert_eq!(lock, Path::new("/run/pmacs/default.sock.lock"));
    }

    #[test]
    fn first_acquire_succeeds_creates_lockfile_with_mode_0600() {
        let tempdir = TempDir::new().unwrap();
        let socket_path = tempdir.path().join("daemon.sock");

        let handle = acquire_lock(&socket_path).expect("first acquire");

        let lockfile_path = lockfile_path_for(&socket_path);
        assert!(lockfile_path.exists());
        let mode = fs::metadata(&lockfile_path).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode, 0o600, "expected 0600, got {mode:#o}");

        // Pid is written advisory.
        let contents = fs::read_to_string(&lockfile_path).unwrap();
        let parsed: u32 = contents.trim().parse().expect("pid in file");
        assert_eq!(parsed, std::process::id());

        handle.release().unwrap();
    }

    #[test]
    fn second_acquire_returns_already_held_with_advisory_pid() {
        let tempdir = TempDir::new().unwrap();
        let socket_path = tempdir.path().join("daemon.sock");

        let _holder = acquire_lock(&socket_path).expect("first acquire");

        match acquire_lock(&socket_path) {
            Err(LockError::AlreadyHeld {
                advisory_pid: Some(pid),
            }) => {
                assert_eq!(pid, std::process::id());
            }
            other => panic!("expected AlreadyHeld with our pid, got {other:?}"),
        }
    }

    #[test]
    fn release_unlinks_lockfile_cleanly() {
        let tempdir = TempDir::new().unwrap();
        let socket_path = tempdir.path().join("daemon.sock");
        let lockfile_path = lockfile_path_for(&socket_path);

        let handle = acquire_lock(&socket_path).expect("acquire");
        assert!(lockfile_path.exists());

        handle.release().unwrap();
        assert!(
            !lockfile_path.exists(),
            "release() should have unlinked {lockfile_path:?}"
        );
    }

    #[test]
    fn drop_releases_lock_so_next_acquire_succeeds() {
        let tempdir = TempDir::new().unwrap();
        let socket_path = tempdir.path().join("daemon.sock");

        {
            let _handle = acquire_lock(&socket_path).expect("first");
            // Drop at end of scope — kernel releases flock as fd closes.
        }

        // Lockfile is still on disk (we didn't release()), but the
        // flock is gone, so the second acquire succeeds.
        let handle2 = acquire_lock(&socket_path).expect("second");
        handle2.release().unwrap();
    }

    #[test]
    fn stale_lockfile_no_holder_succeeds_and_overwrites_pid() {
        // Pre-create a lockfile with a bogus pid string. No process
        // holds a flock on it. The next acquire must succeed and
        // overwrite the bogus pid with our own.
        let tempdir = TempDir::new().unwrap();
        let socket_path = tempdir.path().join("daemon.sock");
        let lockfile_path = lockfile_path_for(&socket_path);

        fs::write(&lockfile_path, b"99999\n").unwrap();
        fs::set_permissions(&lockfile_path, fs::Permissions::from_mode(0o600)).unwrap();

        let handle = acquire_lock(&socket_path).expect("acquire over stale");
        let contents = fs::read_to_string(&lockfile_path).unwrap();
        let parsed: u32 = contents.trim().parse().unwrap();
        assert_eq!(parsed, std::process::id());

        handle.release().unwrap();
    }

    #[test]
    fn concurrent_acquires_only_one_wins() {
        // Spawn a thread that tries to acquire the lock while the
        // main thread holds it, then expect the spawned thread to
        // observe AlreadyHeld with the main thread's pid.
        let tempdir = TempDir::new().unwrap();
        let socket_path = tempdir.path().join("daemon.sock");

        let holder = acquire_lock(&socket_path).expect("main acquires");
        let our_pid = std::process::id();

        let socket_path_clone = socket_path.clone();
        let (tx, rx) = mpsc::channel();
        let join = thread::spawn(move || {
            let result = acquire_lock(&socket_path_clone);
            tx.send(result).unwrap();
        });

        let result = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("thread reports");
        match result {
            Err(LockError::AlreadyHeld { advisory_pid }) => {
                assert_eq!(advisory_pid, Some(our_pid));
            }
            other => panic!("expected AlreadyHeld, got {other:?}"),
        }
        join.join().unwrap();

        // Drop the holder; spawned thread already finished.
        holder.release().unwrap();
    }
}
