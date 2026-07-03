// file_io.rs --- File load and atomic save.

//! File I/O: load a file into bytes; save bytes to a file atomically.
//!
//! # Atomicity
//!
//! [`save_atomic`] writes to a sibling temp file (same parent directory,
//! same filesystem) and renames over the target. POSIX `rename(2)` is
//! atomic on the same filesystem, so a crash mid-write leaves either the
//! old file or the new file --- never a truncated half-file.
//!
//! # External-modification detection
//!
//! [`load_file`] returns a [`FileMeta`] capturing modification time and
//! size at load. Before saving, callers should query
//! [`current_meta`] and compare; a mismatch indicates the file has been
//! changed by another process and the user should be prompted before
//! overwriting.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

/// Attempts to find a free temp-file name before giving up (F-006). The
/// atomic sequence makes same-process names unique, so retries only cover
/// the rare stale-temp-from-a-crashed-run collision.
const MAX_TEMP_ATTEMPTS: u32 = 8;

/// Process-global disambiguator for temp names — guarantees two saves in
/// the same process never collide, even within one nanosecond (F-006).
static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// File metadata
// ---------------------------------------------------------------------------

/// Snapshot of a file's identity for change detection.
///
/// Two `FileMeta`s comparing equal indicates --- with very high probability
/// --- that the file has not been modified between the two queries.
/// `mtime` alone is not enough on filesystems with second-resolution
/// timestamps, so size is included to distinguish edits within the same
/// second.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileMeta {
    /// Modification time, as reported by the filesystem.
    pub mtime: SystemTime,
    /// Size in bytes.
    pub size: u64,
}

impl FileMeta {
    fn from_metadata(meta: &fs::Metadata) -> io::Result<Self> {
        Ok(Self {
            mtime: meta.modified()?,
            size: meta.len(),
        })
    }
}

/// Read the metadata of `path`. Errors if the file does not exist or is
/// inaccessible.
pub fn current_meta(path: &Path) -> io::Result<FileMeta> {
    FileMeta::from_metadata(&fs::metadata(path)?)
}

// ---------------------------------------------------------------------------
// Load
// ---------------------------------------------------------------------------

/// Load `path` into a byte vector and return its metadata snapshot.
///
/// The vector preserves the file's bytes verbatim --- no encoding
/// translation, no line-ending normalization. Round-trip with [`save_atomic`]
/// is byte-identical.
///
/// Threading: any thread.
pub fn load_file(path: &Path) -> io::Result<(Vec<u8>, FileMeta)> {
    let mut file = File::open(path)?;
    let meta = FileMeta::from_metadata(&file.metadata()?)?;
    let mut bytes = Vec::with_capacity(meta.size as usize);
    file.read_to_end(&mut bytes)?;
    Ok((bytes, meta))
}

// ---------------------------------------------------------------------------
// Save
// ---------------------------------------------------------------------------

/// Reasons [`save_atomic`] can fail in addition to plain `io::Error`.
#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    /// The target's parent directory does not exist or could not be
    /// determined. Save targets must have a parent so the temp file can
    /// live alongside.
    #[error("save target has no parent directory: {0}")]
    NoParent(PathBuf),
    /// An underlying I/O error.
    #[error("io error: {0}")]
    Io(#[from] io::Error),
}

/// Cleanup guard: removes the named temp file on drop unless defused.
///
/// Lives at module scope so [`save_atomic`] can use it without confusing
/// item-after-statements lints.
struct TempCleanup {
    /// Path to clean up. Set to `None` to defuse (rename succeeded).
    tmp: Option<PathBuf>,
}

impl Drop for TempCleanup {
    fn drop(&mut self) {
        if let Some(p) = self.tmp.take() {
            let _ = fs::remove_file(&p);
        }
    }
}

/// Atomically write `content` to `path`.
///
/// Creates a sibling temporary file in `path`'s parent directory, writes
/// the content, syncs, and renames over `path`. On error the temp file is
/// best-effort cleaned up.
///
/// Returns the post-write [`FileMeta`] so the caller can record the new
/// identity for future change-detection comparisons.
///
/// Threading: any thread.
pub fn save_atomic(path: &Path, content: &[u8]) -> Result<FileMeta, SaveError> {
    // `Path::parent` returns:
    //  * `None` for `/` or `""` --- no place to put a sibling temp file;
    //  * `Some("")` for a bare filename like `notes.txt` --- means cwd, fine;
    //  * `Some("/foo")` etc. --- the explicit parent directory.
    // Only `None` is a real error.
    if path.parent().is_none() {
        return Err(SaveError::NoParent(path.to_path_buf()));
    }

    // Snapshot the target's current permissions so an existing file keeps
    // its mode across the replace (F-006) — e.g. a `0755` script stays
    // executable. `None` for a new file, which then gets the default mode.
    let existing_perms = fs::metadata(path).ok().map(|m| m.permissions());

    // Open a fresh temp, retrying on the rare name collision (a stale temp
    // left by a crashed prior run whose pid+nanos recurs) instead of
    // failing the save (F-006). `TEMP_SEQ` makes same-process names unique.
    let (mut tmp, tmp_path) = {
        let mut opened = None;
        for _ in 0..MAX_TEMP_ATTEMPTS {
            let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
            let candidate = temp_sibling(path, seq);
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&candidate)
            {
                Ok(file) => {
                    opened = Some((file, candidate));
                    break;
                }
                // Name taken (a stale temp): fall through to the next seq.
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(SaveError::Io(e)),
            }
        }
        opened.ok_or_else(|| {
            SaveError::Io(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "save_atomic: temp name still colliding after retries",
            ))
        })?
    };

    let mut guard = TempCleanup {
        tmp: Some(tmp_path.clone()),
    };

    // Carry the target's mode onto the temp so the saved file preserves it
    // (F-006). Done *before* the write so a sensitive (e.g. 0600) file's
    // content is never briefly world-readable in a default-perms temp; the
    // already-open write handle keeps write access regardless of the new
    // mode (Unix checks permissions at open, not per write).
    if let Some(perms) = existing_perms {
        fs::set_permissions(&tmp_path, perms).map_err(SaveError::Io)?;
    }
    tmp.write_all(content).map_err(SaveError::Io)?;
    tmp.sync_all().map_err(SaveError::Io)?;
    drop(tmp); // close before rename (Windows can't rename an open file)

    fs::rename(&tmp_path, path).map_err(SaveError::Io)?;
    // Rename succeeded: temp no longer exists at tmp_path; defuse cleanup.
    guard.tmp = None;

    // fsync the parent directory so the rename (a directory operation) is
    // durable across a crash, not just the file bytes `sync_all` covered
    // (F-006). Best-effort: the rename already succeeded, and some
    // filesystems reject directory fsync. Directory fsync is a Unix concept.
    #[cfg(unix)]
    sync_parent_dir(path);

    let meta = current_meta(path)?;
    Ok(meta)
}

fn temp_sibling(target: &Path, seq: u64) -> PathBuf {
    // `parent()` may be empty (target is a bare filename in cwd) or
    // non-empty (target lives under some directory). `Path::join` handles
    // both correctly: empty parent + name == name; non-empty parent + name
    // == parent/name.
    let parent = target.parent().expect("checked by caller");
    let mut name = OsString::new();
    name.push(".pmacs-tmp.");
    name.push(target.file_name().unwrap_or_default());
    name.push(".");
    name.push(format!("{:x}", std::process::id()));
    name.push(".");
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    // pid + subsecond nanos + a process-global sequence: the sequence
    // guarantees same-process uniqueness, the rest disambiguates across
    // processes/runs.
    name.push(format!("{nanos:x}.{seq:x}"));
    parent.join(name)
}

/// fsync the directory containing `path` so a rename into it is durable
/// (F-006). Best-effort — errors are ignored (see the call site).
#[cfg(unix)]
fn sync_parent_dir(path: &Path) {
    let Some(parent) = path.parent() else {
        return;
    };
    // An empty parent means the current directory.
    let dir = if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    };
    if let Ok(dir) = File::open(dir) {
        let _ = dir.sync_all();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn round_trip_small_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("hello.txt");
        save_atomic(&path, b"hello world").unwrap();

        let (bytes, meta) = load_file(&path).unwrap();
        assert_eq!(bytes, b"hello world");
        assert_eq!(meta.size, 11);
    }

    #[test]
    fn round_trip_large_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("big.bin");
        let content: Vec<u8> = (0..1_000_000).map(|i| (i % 251) as u8).collect();
        save_atomic(&path, &content).unwrap();

        let (bytes, meta) = load_file(&path).unwrap();
        assert_eq!(bytes, content);
        assert_eq!(meta.size, content.len() as u64);
    }

    #[test]
    fn round_trip_empty_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("empty.txt");
        save_atomic(&path, b"").unwrap();

        let (bytes, meta) = load_file(&path).unwrap();
        assert!(bytes.is_empty());
        assert_eq!(meta.size, 0);
    }

    #[test]
    fn round_trip_arbitrary_bytes() {
        // No encoding translation: every byte from 0..=255 must survive.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("bytes.bin");
        let content: Vec<u8> = (0u32..=255).map(|i| i as u8).collect();
        save_atomic(&path, &content).unwrap();
        let (bytes, _) = load_file(&path).unwrap();
        assert_eq!(bytes, content);
    }

    #[cfg(unix)]
    #[test]
    fn save_preserves_existing_file_mode() {
        // F-006 — an atomic save over an existing file keeps its mode, so
        // a `0755` script stays executable instead of dropping to `0644`.
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("script.sh");
        save_atomic(&path, b"#!/bin/sh\necho hi\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();

        save_atomic(&path, b"#!/bin/sh\necho bye\n").unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755, "the executable bit must survive the save");
    }

    #[test]
    fn temp_sibling_disambiguates_by_sequence() {
        // F-006 — the process-global sequence makes temp names unique even
        // for the same target within one nanosecond.
        let p = Path::new("/tmp/foo.txt");
        assert_ne!(temp_sibling(p, 1), temp_sibling(p, 2));
    }

    #[test]
    fn rapid_saves_in_one_process_do_not_collide() {
        // F-006 — back-to-back saves must never spuriously fail on a temp
        // name collision (the sequence guarantees uniqueness).
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("hot.txt");
        for i in 0..50u32 {
            save_atomic(&path, format!("write {i}").as_bytes()).unwrap();
        }
        let (bytes, _) = load_file(&path).unwrap();
        assert_eq!(bytes, b"write 49");
    }

    #[test]
    fn save_overwrites_existing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("notes.txt");
        save_atomic(&path, b"first").unwrap();
        save_atomic(&path, b"second").unwrap();
        let (bytes, _) = load_file(&path).unwrap();
        assert_eq!(bytes, b"second");
    }

    #[test]
    fn save_does_not_leave_temp_file_on_success() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("ok.txt");
        save_atomic(&path, b"x").unwrap();

        // Walk the directory; only the target file should remain.
        let entries: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0], "ok.txt");
    }

    #[test]
    fn current_meta_matches_save_meta() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.txt");
        let saved = save_atomic(&path, b"abc").unwrap();
        let queried = current_meta(&path).unwrap();
        assert_eq!(saved, queried);
    }

    #[test]
    fn external_modification_detected() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("watch.txt");
        save_atomic(&path, b"original").unwrap();
        let (_bytes, meta_at_load) = load_file(&path).unwrap();

        // Sleep long enough to bump mtime (most filesystems are ms or
        // second-grained; 1.1 s is portable).
        std::thread::sleep(std::time::Duration::from_millis(1_100));
        save_atomic(&path, b"externally changed").unwrap();

        let now = current_meta(&path).unwrap();
        assert_ne!(meta_at_load, now);
    }

    #[test]
    fn bare_filename_saves_in_cwd() {
        // `Path::parent()` of a bare filename returns Some("") (empty path),
        // not None. Treat that as "save in cwd". Run inside a tempdir so we
        // don't litter the workspace.
        let dir = TempDir::new().unwrap();
        let prev_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let result = save_atomic(Path::new("bare.txt"), b"x");
        // Restore cwd before any assertion that might unwind the test.
        std::env::set_current_dir(&prev_cwd).unwrap();
        let meta = result.expect("bare filename should save in cwd");
        assert_eq!(meta.size, 1);

        // Verify the file landed in the tempdir.
        let (bytes, _) = load_file(&dir.path().join("bare.txt")).unwrap();
        assert_eq!(bytes, b"x");
    }

    #[test]
    fn root_path_is_an_error() {
        // `Path::parent()` of "/" is None; that's the real "no parent" case.
        let result = save_atomic(Path::new("/"), b"x");
        assert!(matches!(result, Err(SaveError::NoParent(_))));
    }
}
