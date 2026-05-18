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
use std::time::SystemTime;

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
    let tmp_path = temp_sibling(path);

    let mut guard = TempCleanup {
        tmp: Some(tmp_path.clone()),
    };

    {
        let mut tmp = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)
            .map_err(SaveError::Io)?;
        tmp.write_all(content).map_err(SaveError::Io)?;
        tmp.sync_all().map_err(SaveError::Io)?;
    }

    fs::rename(&tmp_path, path).map_err(SaveError::Io)?;
    // Rename succeeded: temp no longer exists at tmp_path; defuse cleanup.
    guard.tmp = None;

    let meta = current_meta(path)?;
    Ok(meta)
}

fn temp_sibling(target: &Path) -> PathBuf {
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
    // A coarse per-call disambiguator. Collisions are vanishingly unlikely
    // and `create_new` would error if hit.
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    name.push(format!("{nanos:x}"));
    parent.join(name)
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
