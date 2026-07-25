// fs.rs --- Worker-dispatched filesystem primitives (T M8.1).

//! Filesystem operations exposed to packages as worker-dispatched
//! async APIs. The synchronous bodies live here; the
//! [`crate::async_runtime`] module owns dispatch / cancellation /
//! supersede plumbing and calls into [`read_dir_blocking`] (and the
//! single-file siblings landing in M8.1b) from the worker thread.
//!
//! ## Why a separate module
//!
//! The runtime's `dispatch_*` surface is the right place for the
//! cancellation-token poll cadence and the bus reply shape, but the
//! actual `lstat` / `readdir` / `readlink` calls have their own error
//! taxonomy and shape concerns (in particular: dired/wdired need
//! lstat-vs-target separation for symlinks, and stat callers want
//! the same per-entry shape minus the `name`). Keeping the bodies
//! out of `async_runtime.rs` lets that file stay focused on the
//! dispatch contract.
//!
//! ## Cancellation contract
//!
//! [`read_dir_blocking`] polls its [`CancellationToken`] every
//! [`READDIR_CANCEL_POLL_EVERY`] entries. A directory of 10K entries
//! observes cancel within a few hundred microseconds; the per-entry
//! overhead from polling is dominated by the underlying syscalls.
//! On cancel the function returns [`Err(FsError::Cancelled)`] and
//! the runtime translates that into the standard
//! [`crate::async_runtime::ReplyKind::Cancelled`] settled state.

use std::io;
use std::path::Path;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::worker::CancellationToken;

/// Cancel-poll cadence for [`read_dir_blocking`]. Picked so the
/// per-poll branch cost is negligible against the per-entry syscall
/// cost while keeping cancel latency under ~1ms even on huge
/// directories.
const READDIR_CANCEL_POLL_EVERY: usize = 32;

/// How many *consecutive* `readdir` iterator errors a tolerant listing
/// records before giving up and failing (dired Q#DR6).
///
/// [`std::fs::ReadDir`] is not obliged to terminate after yielding an
/// `Err`: a directory pulled out from under a stalled network mount can
/// keep producing them. Tolerant mode records-and-continues, so without
/// a bound that is an unbounded error vector on a worker thread.
///
/// Cancellation is **not** an adequate backstop here, which is the
/// reason this constant exists rather than a comment saying it is: a
/// dired listing carries no supersede key and nothing cancels it, so the
/// only thing that would stop the loop is the directory itself. A
/// directory whose iterator produces nothing but errors has no partial
/// answer worth rendering, so the listing fails with the last error the
/// way an unopenable directory does.
///
/// Deliberately untested: forcing a real `readdir` to yield errors
/// repeatedly is not portable, and faking it would need the walk to be
/// generic over its iterator — a refactor with no other consumer. The
/// counter resets on any entry that materializes.
const READDIR_MAX_CONSECUTIVE_ENTRY_ERRORS: usize = 1024;

/// One directory entry as returned by [`read_dir_blocking`].
///
/// The shape is what `dired` / `magit-class` / `outline-class`
/// packages need without extra Lua-side parsing: `lstat`-style
/// metadata plus, for symlinks, the resolved target as a separate
/// field. Wdired's edit-to-rename layer treats edits to the `name`
/// column as `rename` calls; keeping `symlink_target` separate lets
/// package code display link destinations without confusing them for
/// editable basename bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsDirEntry {
    /// Basename of the entry (no directory prefix).
    pub name: String,
    /// What kind of filesystem object this is, per `lstat`. Symlinks
    /// stay as [`FsEntryKind::Symlink`] regardless of what they
    /// point to; callers that want the resolved kind follow
    /// [`Self::symlink_target`] manually.
    pub kind: FsEntryKind,
    /// Size in bytes from `lstat`. For directories this is the
    /// inode size, not the cumulative tree size.
    pub size: u64,
    /// Modification time as Unix seconds since epoch. `i64` so
    /// pre-1970 timestamps (rare but legal) round-trip without
    /// going negative through `u64`.
    pub mtime_secs: i64,
    /// Nanosecond component of the modification timestamp. Paired
    /// with [`Self::mtime_secs`] so dired/wdired can detect same-size
    /// rewrites that happen within one second.
    pub mtime_nsec: u32,
    /// Permission bits from `lstat`'s `st_mode`, masked to the low
    /// 12 bits (setuid/setgid/sticky + rwx). Higher bits (file
    /// type) are exposed via [`Self::kind`].
    pub mode: u32,
    /// For symlinks, the path the symlink resolves to (a literal
    /// `readlink` result, not canonicalized). `None` for non-symlink
    /// entries. Wdired's symlink-edit path consumes this directly.
    pub symlink_target: Option<String>,
}

/// Discriminator for [`FsDirEntry::kind`].
///
/// `Other` covers device nodes, fifos, sockets, and anything else
/// the underlying `stat` reports. v0.1 packages have no need to
/// distinguish them; if a future package does, we extend the enum
/// and the Lua-side string discriminator with a new variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FsEntryKind {
    /// Regular file.
    File,
    /// Directory.
    Dir,
    /// Symbolic link. [`FsDirEntry::symlink_target`] holds the
    /// target.
    Symlink,
    /// Anything that's not a file/dir/symlink: device, fifo,
    /// socket, etc.
    Other,
}

impl FsEntryKind {
    /// Stable string used at the Lua boundary. Lua callers compare
    /// against literal strings rather than getting an integer
    /// discriminator.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Dir => "dir",
            Self::Symlink => "symlink",
            Self::Other => "other",
        }
    }
}

/// Per-entry tolerance for [`read_dir_blocking`] (dired Q#DR6).
///
/// The M8.1 primitive was all-or-nothing: five per-entry conditions
/// failed the *entire* listing, which makes a plain refresh of a busy
/// directory (`/tmp`, a build tree) fail outright. The module doc used
/// to say a per-entry-tolerant wrapper was "the package's job" --- it
/// cannot be: the primitive hands Lua one structured error and no
/// partial vec, so there is nothing to be tolerant *with*.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadDirTolerance {
    /// Any per-entry failure fails the whole listing. The original
    /// M8.1 contract, and still the default at every Lua call site
    /// that does not opt in.
    Fatal,
    /// Per-entry failures are recorded in [`FsDirListing::errors`] and
    /// enumeration continues. A failure on the *parent* `read_dir`
    /// stays fatal (a directory you cannot open has no partial
    /// answer), and so does a non-UTF-8 entry **name** --- see
    /// [`FsError::NonUtf8Path`].
    PerEntry,
}

/// One per-entry failure recorded by a tolerant [`read_dir_blocking`].
///
/// `name` is optional because a per-entry `readdir` *iterator* error
/// has no filename to report: the entry never materialized, and the
/// underlying error is about the parent directory. Every other arm has
/// an entry in hand and names it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsDirEntryError {
    /// Basename of the entry that failed, when one is known.
    pub name: Option<String>,
    /// Rendered failure, already formatted for display.
    pub message: String,
}

/// What [`read_dir_blocking`] returns: the entries it could read, plus
/// the per-entry failures when the caller asked to tolerate them.
///
/// `errors` is `None` under [`ReadDirTolerance::Fatal`] and `Some`
/// (possibly empty) under [`ReadDirTolerance::PerEntry`]. The
/// distinction is load-bearing at the Lua boundary: it is what selects
/// the bare-array result shape the M8.1 surface promises from the
/// `{ entries = …, errors = … }` shape the tolerant opt returns, so the
/// conversion never has to look the job back up.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsDirListing {
    /// One entry per readable child, in filesystem iteration order.
    pub entries: Vec<FsDirEntry>,
    /// Per-entry failures; `None` in [`ReadDirTolerance::Fatal`] mode.
    pub errors: Option<Vec<FsDirEntryError>>,
}

/// Errors produced by [`read_dir_blocking`] / [`stat_blocking`] /
/// [`rename_blocking`] / [`chmod_blocking`] / [`remove_blocking`].
///
/// `Io` is the catch-all for OS-level failures (path not found,
/// permission denied, etc.). `Cancelled` is the cooperative-cancel
/// path produced when the worker observes its [`CancellationToken`]
/// flipped mid-enumeration. `NonUtf8Path` is the explicit-rejection
/// path for entries whose name (or symlink target) isn't valid
/// UTF-8 --- the v0.1 fs surface doesn't expose byte-preserving
/// paths, and we'd rather fail loudly than silently mangle the
/// name through `to_string_lossy`.
#[derive(Debug, Error)]
pub enum FsError {
    /// Underlying OS error; carries the offending path so error
    /// messages can name what we tried.
    #[error("filesystem operation on `{path}`: {source}")]
    Io {
        /// Path the operation was working on when the error
        /// surfaced.
        path: String,
        /// The OS error.
        #[source]
        source: io::Error,
    },
    /// The cancel token was observed flipped before the operation
    /// completed. No filesystem state has been mutated in this
    /// case; the partial result is discarded.
    #[error("filesystem operation cancelled")]
    Cancelled,
    /// An entry name or symlink target wasn't valid UTF-8. v0.1's
    /// Lua fs surface uses Lua strings via `String`; non-UTF-8
    /// bytes can't round-trip without a wider API. The error
    /// names the parent directory (or the path being statted) and
    /// the offending raw bytes so the user can see what they hit.
    /// Byte-preserving paths are post-v0.1 work; if a real M8
    /// package needs them, we widen the surface to accept and
    /// return Lua strings (which are byte arrays at the C layer).
    #[error(
        "non-UTF-8 filesystem name in `{parent}`: raw bytes {bytes:?}; \
         pmacs.fs v0.1 requires UTF-8 names. Rename the offending entry \
         or open an issue if you need byte-preserving paths."
    )]
    NonUtf8Path {
        /// Parent directory (for `read_dir`) or path being
        /// statted (for `stat`'s symlink target case).
        parent: String,
        /// Raw bytes of the offending name, for diagnostic
        /// display.
        bytes: Vec<u8>,
    },
}

/// Synchronous body of `read_dir`: enumerate `path` and return one
/// [`FsDirEntry`] per child. Polls `cancel` every
/// [`READDIR_CANCEL_POLL_EVERY`] entries; returns
/// [`FsError::Cancelled`] if the token is flipped mid-walk.
///
/// Each entry's `kind` and metadata come from `lstat` (not `stat`),
/// so symlinks are reported as symlinks regardless of what they
/// point to. For symlink entries we additionally call `readlink` to
/// fill in [`FsDirEntry::symlink_target`].
///
/// **Ordering:** the returned vec is in *filesystem iteration
/// order* (whatever `readdir(3)` produces). The primitive
/// intentionally doesn't sort: dired-class packages (M8.2) own
/// user-facing sort modes per the spec, and forcing a sort here
/// would either pick one arbitrarily or pay for sorting twice
/// when the package then sorts by its own criterion.
///
/// **UTF-8 constraint:** entry names and symlink targets must be
/// valid UTF-8. A non-UTF-8 entry surfaces as
/// [`FsError::NonUtf8Path`] naming the parent and the offending
/// bytes; we don't silently lossy-convert (the prior
/// `to_string_lossy` would have mangled dired/wdired round-trips).
///
/// Errors on the *parent* `read_dir` call surface as
/// [`FsError::Io`] regardless of `tolerance`: a directory you cannot
/// open has no partial answer.
///
/// Errors on individual entries (a permission-denied `lstat`, a child
/// unlinked between `readdir` and `lstat`, a `readlink` failure, a
/// non-UTF-8 symlink target) are governed by `tolerance`. Under
/// [`ReadDirTolerance::Fatal`] they fail the whole listing, which is
/// the M8.1 contract every existing caller relies on; under
/// [`ReadDirTolerance::PerEntry`] they land in
/// [`FsDirListing::errors`] and enumeration continues (dired Q#DR6).
///
/// A non-UTF-8 entry **name** is fatal in both modes. That is not a
/// listing problem but a path-representation one: [`FsDirEntry::name`]
/// is a `String` and every `pmacs.fs` op takes a `String` path, so a
/// tolerantly-rendered non-UTF-8 name would be a name the caller could
/// not pass back through `rename`. Byte-preserving paths are the named
/// deferral (see [`FsError::NonUtf8Path`]). A non-UTF-8 *target*
/// differs in kind --- the entry's own name is fine and nothing needs
/// to round-trip the target --- so it joins the per-entry channel.
pub fn read_dir_blocking(
    path: &Path,
    cancel: &CancellationToken,
    tolerance: ReadDirTolerance,
) -> Result<FsDirListing, FsError> {
    let iter = std::fs::read_dir(path).map_err(|source| FsError::Io {
        path: path.display().to_string(),
        source,
    })?;
    let mut out: Vec<FsDirEntry> = Vec::new();
    let mut errors: Option<Vec<FsDirEntryError>> =
        matches!(tolerance, ReadDirTolerance::PerEntry).then(Vec::new);
    let parent_str = path.display().to_string();
    let mut consecutive_entry_errors = 0usize;
    for (i, entry_result) in iter.enumerate() {
        if i % READDIR_CANCEL_POLL_EVERY == 0 && cancel.is_cancelled() {
            return Err(FsError::Cancelled);
        }
        let entry = match entry_result {
            Ok(entry) => entry,
            Err(source) => {
                // R2-2: the entry never materialized, so there is no
                // name to report and the error names the parent.
                let error = FsError::Io {
                    path: parent_str.clone(),
                    source,
                };
                consecutive_entry_errors += 1;
                if consecutive_entry_errors > READDIR_MAX_CONSECUTIVE_ENTRY_ERRORS {
                    return Err(error);
                }
                record_entry_error(&mut errors, None, error)?;
                continue;
            }
        };
        consecutive_entry_errors = 0;
        let entry_path = entry.path();
        // Resolved first so a later per-entry failure can name it.
        let name = path_to_utf8_string(&entry.file_name(), &parent_str)?;
        let metadata = match std::fs::symlink_metadata(&entry_path) {
            Ok(metadata) => metadata,
            Err(source) => {
                record_entry_error(
                    &mut errors,
                    Some(&name),
                    FsError::Io {
                        path: entry_path.display().to_string(),
                        source,
                    },
                )?;
                continue;
            }
        };
        let kind = classify(&metadata);
        let mut symlink_target = None;
        if matches!(kind, FsEntryKind::Symlink) {
            match std::fs::read_link(&entry_path) {
                // A target we cannot represent leaves the entry in the
                // listing with its target unknown, not the entry out of
                // it: one weird symlink in `/tmp` used to take the
                // whole directory down.
                Ok(target) => match path_to_utf8_string(target.as_os_str(), &parent_str) {
                    Ok(target) => symlink_target = Some(target),
                    Err(error) => record_entry_error(&mut errors, Some(&name), error)?,
                },
                Err(source) => record_entry_error(
                    &mut errors,
                    Some(&name),
                    FsError::Io {
                        path: entry_path.display().to_string(),
                        source,
                    },
                )?,
            }
        }
        out.push(FsDirEntry {
            name,
            kind,
            size: metadata.len(),
            mtime_secs: mtime_to_unix_secs(&metadata),
            mtime_nsec: mtime_to_unix_nsec(&metadata),
            mode: mode_bits(&metadata),
            symlink_target,
        });
    }
    Ok(FsDirListing {
        entries: out,
        errors,
    })
}

/// Route one per-entry failure: append it to the tolerant channel, or
/// propagate it when the caller asked for the fatal contract.
///
/// `errors.is_none()` *is* [`ReadDirTolerance::Fatal`] --- keeping the
/// mode in the accumulator rather than passing it separately makes the
/// two impossible to disagree.
fn record_entry_error(
    errors: &mut Option<Vec<FsDirEntryError>>,
    name: Option<&str>,
    error: FsError,
) -> Result<(), FsError> {
    match errors {
        Some(list) => {
            list.push(FsDirEntryError {
                name: name.map(ToOwned::to_owned),
                message: error.to_string(),
            });
            Ok(())
        }
        None => Err(error),
    }
}

/// Convert an [`std::ffi::OsStr`] to `String` strictly. Returns
/// [`FsError::NonUtf8Path`] (named with `parent_for_error` as the
/// directory the entry came from) when the name isn't valid UTF-8.
/// This is the M#3 fix for the M8.1 review: the prior
/// `to_string_lossy` silently mangled non-UTF-8 names, which broke
/// any dired/wdired round-trip that touched them.
fn path_to_utf8_string(name: &std::ffi::OsStr, parent_for_error: &str) -> Result<String, FsError> {
    if let Some(s) = name.to_str() {
        Ok(s.to_string())
    } else {
        Err(FsError::NonUtf8Path {
            parent: parent_for_error.to_string(),
            bytes: os_str_bytes(name),
        })
    }
}

/// Get the raw bytes of an `OsStr`. Unix-only --- pmacs ships only
/// on Unix in v0.1 and the M8 packages are designed for it. The
/// non-Unix branch returns the lossy form purely so the diagnostic
/// message has *something* to display; pmacs's audit forbids
/// shipping a non-Unix build that would actually take this path.
fn os_str_bytes(s: &std::ffi::OsStr) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        s.as_bytes().to_vec()
    }
    #[cfg(not(unix))]
    {
        s.to_string_lossy().into_owned().into_bytes()
    }
}

/// Synchronous body of `stat`: returns metadata for a single path.
/// Like [`read_dir_blocking`], the metadata comes from `lstat`, not
/// `stat` --- a symlink is reported as a symlink with its target in
/// [`FsDirEntry::symlink_target`], not as the target's metadata.
/// `name` is the basename of `path`; callers already hold the full
/// path, but the basename keeps the result shape symmetric with
/// `read_dir`'s entries so packages can treat a stat result and a
/// `read_dir` entry interchangeably.
///
/// Cancel token is polled once at entry; the syscall itself is
/// non-cancellable but completes quickly enough that mid-syscall
/// cancellation isn't a meaningful concept here.
pub fn stat_blocking(path: &Path, cancel: &CancellationToken) -> Result<FsDirEntry, FsError> {
    if cancel.is_cancelled() {
        return Err(FsError::Cancelled);
    }
    let path_str = path.display().to_string();
    let metadata = std::fs::symlink_metadata(path).map_err(|source| FsError::Io {
        path: path_str.clone(),
        source,
    })?;
    let kind = classify(&metadata);
    let symlink_target = if matches!(kind, FsEntryKind::Symlink) {
        match std::fs::read_link(path) {
            Ok(t) => Some(path_to_utf8_string(t.as_os_str(), &path_str)?),
            Err(source) => {
                return Err(FsError::Io {
                    path: path_str.clone(),
                    source,
                });
            }
        }
    } else {
        None
    };
    let name = match path.file_name() {
        Some(s) => path_to_utf8_string(s, &path_str)?,
        None => String::new(),
    };
    Ok(FsDirEntry {
        name,
        kind,
        size: metadata.len(),
        mtime_secs: mtime_to_unix_secs(&metadata),
        mtime_nsec: mtime_to_unix_nsec(&metadata),
        mode: mode_bits(&metadata),
        symlink_target,
    })
}

/// Synchronous body of `rename`: atomic on-disk rename of `from`
/// to `to`. Cross-filesystem renames fall through to the OS
/// behavior (Linux returns EXDEV; the caller can either copy+remove
/// at the package layer or wait for a future `pmacs.fs.move`
/// primitive that handles the cross-fs case).
pub fn rename_blocking(from: &Path, to: &Path, cancel: &CancellationToken) -> Result<(), FsError> {
    if cancel.is_cancelled() {
        return Err(FsError::Cancelled);
    }
    std::fs::rename(from, to).map_err(|source| FsError::Io {
        path: format!("{} -> {}", from.display(), to.display()),
        source,
    })
}

/// Synchronous body of `chmod`: replace `path`'s permission bits
/// with the low 12 bits of `mode`. Higher bits (file type) are
/// silently ignored; callers shouldn't be passing them.
///
/// **Symlink semantics:** chmod on a symlink follows the link and
/// modifies the *target*, per the standard `chmod(2)` syscall.
/// This is asymmetric with [`read_dir_blocking`] /
/// [`stat_blocking`], which use `lstat` and report the link's own
/// metadata. Concrete consequence for dired/wdired: chmodding a
/// symlink line in the buffer changes the target's permission
/// bits, but a refresh of that line shows the link's own
/// (unchanged) mode --- on most filesystems a symlink reports
/// `0o777` regardless of what was done through it.
///
/// Packages that need lchmod-style behavior (modify the link
/// itself, not the target) need a platform-specific syscall not
/// portable across Unixes; v0.1's pmacs.fs surface picks the
/// portable `chmod(2)` shape and documents the asymmetry rather
/// than papering over it.
pub fn chmod_blocking(path: &Path, mode: u32, cancel: &CancellationToken) -> Result<(), FsError> {
    if cancel.is_cancelled() {
        return Err(FsError::Cancelled);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(mode & 0o7777);
        std::fs::set_permissions(path, perms).map_err(|source| FsError::Io {
            path: path.display().to_string(),
            source,
        })
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
        Err(FsError::Io {
            path: path.display().to_string(),
            source: io::Error::new(io::ErrorKind::Unsupported, "chmod is Unix-only"),
        })
    }
}

/// Synchronous body of `remove`: delete a single filesystem object.
/// Inspects `lstat` metadata first to dispatch to `remove_file` or
/// `remove_dir` explicitly --- avoids depending on a specific errno
/// (Linux's `EISDIR` is 21; other Unix platforms can return
/// different values for "tried to unlink a directory"). Callers
/// that need to delete a non-empty directory walk it with
/// `read_dir` and remove children before the parent --- recursive
/// deletion is left to the package layer because the policy
/// ("confirm? skip on error? halt?") belongs there, not at the
/// primitive.
///
/// Symlinks are removed as symlinks: the `lstat` reports
/// [`FsEntryKind::Symlink`] regardless of the target's kind, and
/// `remove_file` on a symlink unlinks the link, not the target.
/// That matters for dired/wdired: a user deleting a symlink line
/// in the buffer should remove the link, not the file it points
/// at.
pub fn remove_blocking(path: &Path, cancel: &CancellationToken) -> Result<(), FsError> {
    if cancel.is_cancelled() {
        return Err(FsError::Cancelled);
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|source| FsError::Io {
        path: path.display().to_string(),
        source,
    })?;
    let result = if metadata.file_type().is_dir() {
        // Real directory --- non-recursive remove. Symlinks-to-dirs
        // are NOT directories per `lstat`, so they fall through to
        // the unlink path below (which removes the link, not the
        // target).
        std::fs::remove_dir(path)
    } else {
        std::fs::remove_file(path)
    };
    result.map_err(|source| FsError::Io {
        path: path.display().to_string(),
        source,
    })
}

fn classify(meta: &std::fs::Metadata) -> FsEntryKind {
    let ft = meta.file_type();
    if ft.is_symlink() {
        FsEntryKind::Symlink
    } else if ft.is_dir() {
        FsEntryKind::Dir
    } else if ft.is_file() {
        FsEntryKind::File
    } else {
        FsEntryKind::Other
    }
}

/// Extract permission bits from `Metadata`. Unix-only for now; on
/// Windows the high bits are zero and the low bits approximate the
/// `chmod`able subset. Pmacs only ships on Unix in v0.1, so the
/// `cfg(unix)` branch is the only one that matters.
fn mode_bits(meta: &std::fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o7777
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        0
    }
}

fn mtime_to_unix_secs(meta: &std::fs::Metadata) -> i64 {
    match meta.modified() {
        Ok(time) => match time.duration_since(SystemTime::UNIX_EPOCH) {
            Ok(d) => i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
            Err(e) => {
                // Pre-epoch timestamp; signed seconds since epoch is
                // negative.
                let neg = e.duration().as_secs();
                -i64::try_from(neg).unwrap_or(i64::MAX)
            }
        },
        Err(_) => 0,
    }
}

fn mtime_to_unix_nsec(meta: &std::fs::Metadata) -> u32 {
    match meta.modified() {
        Ok(time) => match time.duration_since(SystemTime::UNIX_EPOCH) {
            Ok(d) => d.subsec_nanos(),
            Err(e) => e.duration().subsec_nanos(),
        },
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::CancellationToken;
    use std::os::unix::fs::symlink;

    fn token() -> CancellationToken {
        CancellationToken::new()
    }

    /// The fatal-mode shorthand every pre-Q#DR6 test used.
    fn read_dir_fatal(path: &Path, cancel: &CancellationToken) -> Result<Vec<FsDirEntry>, FsError> {
        read_dir_blocking(path, cancel, ReadDirTolerance::Fatal).map(|listing| {
            assert!(
                listing.errors.is_none(),
                "fatal mode must not open a per-entry channel"
            );
            listing.entries
        })
    }

    #[test]
    fn read_dir_returns_entries_with_lstat_metadata() {
        let td = tempfile::tempdir().expect("tempdir");
        std::fs::write(td.path().join("a.txt"), b"hello").expect("write");
        std::fs::create_dir(td.path().join("subdir")).expect("mkdir");
        let entries = read_dir_fatal(td.path(), &token()).expect("read_dir");
        let mut names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, vec!["a.txt", "subdir"]);
        let a = entries.iter().find(|e| e.name == "a.txt").unwrap();
        assert_eq!(a.kind, FsEntryKind::File);
        assert_eq!(a.size, 5);
        let s = entries.iter().find(|e| e.name == "subdir").unwrap();
        assert_eq!(s.kind, FsEntryKind::Dir);
    }

    #[test]
    fn read_dir_reports_symlink_with_separate_target() {
        let td = tempfile::tempdir().expect("tempdir");
        std::fs::write(td.path().join("real.txt"), b"x").expect("write");
        symlink("real.txt", td.path().join("link")).expect("symlink");
        let entries = read_dir_fatal(td.path(), &token()).expect("read_dir");
        let link = entries.iter().find(|e| e.name == "link").unwrap();
        assert_eq!(link.kind, FsEntryKind::Symlink);
        assert_eq!(link.symlink_target.as_deref(), Some("real.txt"));
        let real = entries.iter().find(|e| e.name == "real.txt").unwrap();
        assert_eq!(real.kind, FsEntryKind::File);
        assert!(real.symlink_target.is_none());
    }

    #[test]
    fn read_dir_polls_cancellation_token() {
        let td = tempfile::tempdir().expect("tempdir");
        // Populate with enough entries that the cancel-poll
        // boundary is crossed before the walk completes.
        for i in 0..(READDIR_CANCEL_POLL_EVERY * 4) {
            std::fs::write(td.path().join(format!("f{i}")), b"").expect("write");
        }
        let cancel = token();
        cancel.cancel();
        let err = read_dir_fatal(td.path(), &cancel).expect_err("must observe cancel");
        assert!(matches!(err, FsError::Cancelled), "got {err:?}");
    }

    #[test]
    fn stat_returns_metadata_for_a_single_path() {
        let td = tempfile::tempdir().expect("tempdir");
        let p = td.path().join("file.txt");
        std::fs::write(&p, b"hello").expect("write");
        let entry = stat_blocking(&p, &token()).expect("stat");
        assert_eq!(entry.name, "file.txt");
        assert_eq!(entry.kind, FsEntryKind::File);
        assert_eq!(entry.size, 5);
        assert!(entry.symlink_target.is_none());
    }

    #[test]
    fn stat_on_symlink_reports_symlink_kind_and_target() {
        let td = tempfile::tempdir().expect("tempdir");
        std::fs::write(td.path().join("real.txt"), b"x").expect("write");
        symlink("real.txt", td.path().join("link")).expect("symlink");
        let entry = stat_blocking(&td.path().join("link"), &token()).expect("stat");
        assert_eq!(entry.kind, FsEntryKind::Symlink);
        assert_eq!(entry.symlink_target.as_deref(), Some("real.txt"));
    }

    #[test]
    fn rename_moves_a_file() {
        let td = tempfile::tempdir().expect("tempdir");
        let from = td.path().join("a.txt");
        let to = td.path().join("b.txt");
        std::fs::write(&from, b"x").expect("write");
        rename_blocking(&from, &to, &token()).expect("rename");
        assert!(!from.exists());
        assert!(to.exists());
    }

    #[test]
    fn chmod_changes_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let td = tempfile::tempdir().expect("tempdir");
        let p = td.path().join("f.txt");
        std::fs::write(&p, b"x").expect("write");
        chmod_blocking(&p, 0o600, &token()).expect("chmod");
        let bits = std::fs::metadata(&p).expect("stat").permissions().mode() & 0o7777;
        assert_eq!(bits, 0o600);
    }

    #[test]
    fn remove_deletes_a_file() {
        let td = tempfile::tempdir().expect("tempdir");
        let p = td.path().join("f.txt");
        std::fs::write(&p, b"x").expect("write");
        remove_blocking(&p, &token()).expect("remove");
        assert!(!p.exists());
    }

    #[test]
    fn remove_falls_through_to_remove_dir_for_empty_directories() {
        let td = tempfile::tempdir().expect("tempdir");
        let dir = td.path().join("empty");
        std::fs::create_dir(&dir).expect("mkdir");
        remove_blocking(&dir, &token()).expect("remove dir");
        assert!(!dir.exists());
    }

    #[test]
    fn remove_on_nonempty_directory_surfaces_io_error() {
        let td = tempfile::tempdir().expect("tempdir");
        let dir = td.path().join("populated");
        std::fs::create_dir(&dir).expect("mkdir");
        std::fs::write(dir.join("child"), b"x").expect("write child");
        let err = remove_blocking(&dir, &token()).expect_err("must fail");
        assert!(matches!(err, FsError::Io { .. }), "got {err:?}");
    }

    #[test]
    fn remove_of_symlink_leaves_target_in_place() {
        let td = tempfile::tempdir().expect("tempdir");
        let real = td.path().join("real.txt");
        let link = td.path().join("link");
        std::fs::write(&real, b"x").expect("write");
        symlink("real.txt", &link).expect("symlink");
        remove_blocking(&link, &token()).expect("remove link");
        assert!(!link.exists(), "link must be gone");
        assert!(real.exists(), "target must survive");
    }

    #[test]
    fn read_dir_on_missing_path_reports_io_error() {
        let td = tempfile::tempdir().expect("tempdir");
        let missing = td.path().join("does-not-exist");
        let err = read_dir_fatal(&missing, &token()).expect_err("must error");
        match err {
            FsError::Io { path, .. } => {
                assert!(
                    path.contains("does-not-exist"),
                    "error must name the path: {path}"
                );
            }
            FsError::Cancelled => panic!("expected Io, got Cancelled"),
            FsError::NonUtf8Path { .. } => panic!("expected Io, got NonUtf8Path"),
        }
    }

    #[test]
    fn read_dir_tolerant_opens_an_empty_error_channel_on_a_clean_directory() {
        // `Some(vec![])` rather than `None` is the whole shape
        // contract: the Lua boundary keys the bare-array-vs-table
        // result on `errors.is_some()`, so a clean tolerant listing
        // must still carry the channel.
        let td = tempfile::tempdir().expect("tempdir");
        std::fs::write(td.path().join("a.txt"), b"x").expect("write");
        let listing = read_dir_blocking(td.path(), &token(), ReadDirTolerance::PerEntry)
            .expect("tolerant read_dir");
        assert_eq!(listing.entries.len(), 1);
        assert_eq!(listing.errors.as_deref(), Some(&[][..]));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn read_dir_tolerant_keeps_an_entry_whose_symlink_target_is_not_utf8() {
        use std::os::unix::ffi::OsStrExt;
        let td = tempfile::tempdir().expect("tempdir");
        std::fs::write(td.path().join("real.txt"), b"x").expect("write");
        // A legal Unix symlink target that is not representable as a
        // Rust `String`. Before Q#DR6 this single entry took the whole
        // listing down.
        symlink(
            std::ffi::OsStr::from_bytes(b"tgt-\xff"),
            td.path().join("weird"),
        )
        .expect("symlink");

        let listing = read_dir_blocking(td.path(), &token(), ReadDirTolerance::PerEntry)
            .expect("tolerant read_dir must survive a non-UTF-8 target");
        let weird = listing
            .entries
            .iter()
            .find(|e| e.name == "weird")
            .expect("the entry itself must be listed");
        assert_eq!(weird.kind, FsEntryKind::Symlink);
        assert!(
            weird.symlink_target.is_none(),
            "an unrepresentable target reports as unknown"
        );
        assert!(
            listing.entries.iter().any(|e| e.name == "real.txt"),
            "the readable sibling must survive too"
        );
        let errors = listing.errors.expect("tolerant mode opens the channel");
        assert_eq!(errors.len(), 1, "one per-entry failure: {errors:?}");
        assert_eq!(errors[0].name.as_deref(), Some("weird"));

        // The same directory under the fatal contract still fails
        // whole-listing --- the opt is what changes behavior, not the
        // walk.
        let err = read_dir_fatal(td.path(), &token()).expect_err("fatal mode must still fail");
        assert!(
            matches!(err, FsError::NonUtf8Path { .. }),
            "expected NonUtf8Path, got {err:?}"
        );
    }

    #[test]
    fn read_dir_tolerant_records_a_failed_lstat_and_lists_nothing_else_wrong() {
        use std::os::unix::fs::PermissionsExt;
        // Failure mode 1 from the framing: a directory readable but not
        // searchable. `readdir` yields the names; every child `lstat`
        // fails with EACCES.
        let td = tempfile::tempdir().expect("tempdir");
        let dir = td.path().join("no-search");
        std::fs::create_dir(&dir).expect("mkdir");
        std::fs::write(dir.join("child"), b"x").expect("write child");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o400)).expect("chmod 400");
        let searchable = std::fs::symlink_metadata(dir.join("child")).is_ok();
        if searchable {
            // Running as root (or on a filesystem that ignores the
            // bits): the premise cannot be established, so assert
            // nothing rather than pass vacuously.
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
                .expect("restore perms");
            eprintln!("lstat still succeeds without search permission; skipping");
            return;
        }

        let tolerant = read_dir_blocking(&dir, &token(), ReadDirTolerance::PerEntry);
        let fatal = read_dir_fatal(&dir, &token());
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .expect("restore perms");

        let listing = tolerant.expect("tolerant read_dir must not fail the listing");
        assert!(
            listing.entries.is_empty(),
            "the unreadable child cannot be described: {:?}",
            listing.entries
        );
        let errors = listing.errors.expect("tolerant mode opens the channel");
        assert_eq!(errors.len(), 1, "one per-entry failure: {errors:?}");
        assert_eq!(
            errors[0].name.as_deref(),
            Some("child"),
            "an lstat failure has an entry in hand and must name it"
        );
        let err = fatal.expect_err("fatal mode must still fail the whole listing");
        assert!(matches!(err, FsError::Io { .. }), "got {err:?}");
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn read_dir_on_non_utf8_entry_name_reports_structured_error() {
        use std::os::unix::ffi::OsStrExt;
        let td = tempfile::tempdir().expect("tempdir");
        // 0xFF is invalid as a UTF-8 leading byte; this is a
        // perfectly legal Unix filename but not representable as
        // Rust `String`.
        let bad_name = std::ffi::OsStr::from_bytes(b"bad-\xff-name");
        std::fs::write(td.path().join(bad_name), b"").expect("write entry");
        let err = read_dir_fatal(td.path(), &token()).expect_err("must error on non-UTF-8");
        match err {
            FsError::NonUtf8Path { parent, bytes } => {
                assert!(
                    parent.contains(td.path().to_string_lossy().as_ref()),
                    "error must name the parent dir: {parent}"
                );
                assert!(
                    bytes.contains(&0xff),
                    "error must carry the offending raw bytes: {bytes:?}"
                );
            }
            other => panic!("expected NonUtf8Path, got {other:?}"),
        }
    }
}
