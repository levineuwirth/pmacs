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
//! # Durability
//!
//! [`save_atomic`] also syncs, so the new file is on the disk when it
//! returns. [`save_atomic_with_mode`] lets a caller say otherwise
//! ([`Durability`]), for state that is cheap to lose and written where a
//! wait on the disk would show.
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

/// What [`save_atomic_with_mode`] waits for before it returns.
///
/// Either way the replace is atomic against a crash of the editor: the
/// rename installs the whole new file or leaves the old one, and what
/// was written sits in the kernel's cache, which a process crash does
/// not lose. The two differ only on a crash of the machine (power, a
/// kernel panic) before the kernel writes that cache back on its own
/// schedule; Linux expires dirty data after thirty seconds by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Durability {
    /// Sync the temp file before the rename and the directory after it
    /// (F-006), so the new contents survive a machine crash once the
    /// call returns. A user's file, and autosave's recovery copy of
    /// one. Each sync waits on the disk without a bound: seconds under
    /// load (#360).
    Synced,
    /// Sync nothing, so the call waits for no sync. After a machine
    /// crash inside the window above, the file holds its old contents,
    /// or, on a filesystem that does not write a file's data before a
    /// rename that replaces another with it, it may be empty. For state
    /// that is cheap to lose (#360).
    Unsynced,
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
///
/// # Errors
/// See [`SaveError`].
pub fn save_atomic(path: &Path, content: &[u8]) -> Result<FileMeta, SaveError> {
    save_atomic_with_mode(path, content, None, Durability::Synced)
}

/// [`save_atomic`], but forcing the target's Unix mode to `mode` instead
/// of inheriting the existing file's mode (or the umask default for a new
/// file), and syncing only as `durability` says.
///
/// The mode is applied to the **temp file before the rename**, so the
/// target never exists — not even momentarily — at a laxer mode. A
/// `chmod` after the write would leave exactly that window, which matters
/// because autosave (Arc 3 Q#AS11) stores *unsaved file contents*: a
/// recovery copy must never be briefly world-readable.
///
/// `mode` is ignored on non-Unix platforms (the write is still atomic).
///
/// # Errors
/// See [`SaveError`].
pub fn save_atomic_with_mode(
    path: &Path,
    content: &[u8],
    mode: Option<u32>,
    durability: Durability,
) -> Result<FileMeta, SaveError> {
    stage_with_mode(path, content, mode)?.finish(durability)
}

/// The first half of [`save_atomic_with_mode`]: `content` written to a
/// fresh sibling temporary file at the target's mode, and nothing synced
/// or renamed. [`Staged::finish`] is the second half, so the two in a
/// row make the same calls in the same order as the whole.
///
/// What the split is for (#365): autosave writes a quit's recovery
/// copies before quit asks its question, where a failure to write one
/// can be named in the question, and syncs and installs them once the
/// question is on the screen.
///
/// # Errors
/// See [`SaveError`]; nothing is left behind on an error.
pub fn stage_with_mode(
    path: &Path,
    content: &[u8],
    mode: Option<u32>,
) -> Result<Staged, SaveError> {
    // `Path::parent` returns:
    //  * `None` for `/` or `""` --- no place to put a sibling temp file;
    //  * `Some("")` for a bare filename like `notes.txt` --- means cwd, fine;
    //  * `Some("/foo")` etc. --- the explicit parent directory.
    // Only `None` is a real error.
    if path.parent().is_none() {
        return Err(SaveError::NoParent(path.to_path_buf()));
    }

    // An explicit `mode` wins; otherwise snapshot the target's current
    // permissions so an existing file keeps its mode across the replace
    // (F-006) — e.g. a `0755` script stays executable. `None` for a new
    // file, which then gets the default mode.
    #[cfg(unix)]
    let existing_perms = mode
        .map(|m| {
            use std::os::unix::fs::PermissionsExt as _;
            fs::Permissions::from_mode(m)
        })
        .or_else(|| fs::metadata(path).ok().map(|m| m.permissions()));
    #[cfg(not(unix))]
    let existing_perms = {
        let _ = mode; // no mode concept; the write is still atomic
        fs::metadata(path).ok().map(|m| m.permissions())
    };

    // Open a fresh temp, retrying on the rare name collision (a stale temp
    // left by a crashed prior run whose pid+nanos recurs) instead of
    // failing the save (F-006). `TEMP_SEQ` makes same-process names unique.
    let (file, tmp_path) = {
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

    // From here an error drops `staged`, which removes the temp.
    let mut staged = Staged {
        file: Some(file),
        tmp: Some(tmp_path),
        target: path.to_path_buf(),
    };
    let tmp_path = staged.tmp.as_deref().expect("just set");

    // Carry the target's mode onto the temp so the saved file preserves it
    // (F-006). Done *before* the write so a sensitive (e.g. 0600) file's
    // content is never briefly world-readable in a default-perms temp; the
    // already-open write handle keeps write access regardless of the new
    // mode (Unix checks permissions at open, not per write).
    if let Some(perms) = existing_perms {
        fs::set_permissions(tmp_path, perms).map_err(SaveError::Io)?;
    }
    staged
        .file
        .as_mut()
        .expect("just set")
        .write_all(content)
        .map_err(SaveError::Io)?;
    Ok(staged)
}

/// Content written beside its target by [`stage_with_mode`] and not yet
/// in its place. [`Staged::finish`] installs it; dropping it unfinished
/// removes the temporary file and leaves the target as it was.
#[derive(Debug)]
pub struct Staged {
    /// The temporary file's write handle, open until [`Staged::finish`]
    /// has synced through it, as the whole save always did.
    file: Option<File>,
    /// The temporary file; `None` once renamed over the target.
    tmp: Option<PathBuf>,
    target: PathBuf,
}

impl Staged {
    /// The path the content is installed at.
    #[must_use]
    pub fn target(&self) -> &Path {
        &self.target
    }

    /// Sync as `durability` says and rename over the target: the temp
    /// file before the rename and the directory after it for
    /// [`Durability::Synced`] (F-006), nothing for
    /// [`Durability::Unsynced`].
    ///
    /// # Errors
    /// See [`SaveError`]. On an error before the rename the temporary
    /// file is removed and the target is untouched.
    pub fn finish(mut self, durability: Durability) -> Result<FileMeta, SaveError> {
        let file = self.file.take().expect("a staged file is finished once");
        if durability == Durability::Synced {
            file.sync_all().map_err(SaveError::Io)?;
        }
        drop(file); // close before rename (Windows can't rename an open file)

        let tmp = self.tmp.as_deref().expect("a staged file is finished once");
        fs::rename(tmp, &self.target).map_err(SaveError::Io)?;
        // Rename succeeded: temp no longer exists at tmp; nothing to remove.
        self.tmp = None;

        // fsync the parent directory so the rename (a directory operation) is
        // durable across a crash, not just the file bytes `sync_all` covered
        // (F-006). Best-effort: the rename already succeeded, and some
        // filesystems reject directory fsync. Directory fsync is a Unix concept.
        #[cfg(unix)]
        if durability == Durability::Synced {
            sync_parent_dir(&self.target);
        }

        let meta = current_meta(&self.target)?;
        Ok(meta)
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        // Close before removing, as the whole save did.
        drop(self.file.take());
        if let Some(p) = self.tmp.take() {
            let _ = fs::remove_file(&p);
        }
    }
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

    #[cfg(unix)]
    #[test]
    fn an_unsynced_save_still_replaces_whole_and_keeps_the_mode() {
        // #360 — `Durability::Unsynced` drops the syncs and nothing else:
        // the content replaced whole, the forced mode on the target, no
        // temp left beside it.
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("places");
        save_atomic_with_mode(&path, b"old", Some(0o600), Durability::Unsynced).unwrap();
        save_atomic_with_mode(&path, b"new", Some(0o600), Durability::Unsynced).unwrap();
        let (bytes, _) = load_file(&path).unwrap();
        assert_eq!(bytes, b"new");
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let entries: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(entries, ["places"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_staged_write_leaves_the_target_until_finished() {
        // #365 — staging writes the temp at the forced mode and touches
        // the target not at all; finishing installs it whole.
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("copy");
        fs::write(&path, b"old").unwrap();
        let staged = stage_with_mode(&path, b"new", Some(0o600)).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"old");
        let temps: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p != &path)
            .collect();
        assert_eq!(temps.len(), 1, "one temp beside the target: {temps:?}");
        let mode = fs::metadata(&temps[0]).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "the temp is at the forced mode before it is installed"
        );
        assert_eq!(staged.target(), path);
        staged.finish(Durability::Synced).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        let entries: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(entries, ["copy"]);
    }

    #[test]
    fn a_staged_write_dropped_unfinished_leaves_the_target_and_no_temp() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("copy");
        fs::write(&path, b"old").unwrap();
        drop(stage_with_mode(&path, b"new", None).unwrap());
        assert_eq!(fs::read(&path).unwrap(), b"old");
        let entries: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(entries, ["copy"]);
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

    /// The env var the parent sets so the child body refuses to run as a
    /// stray `--ignored` selection. Named, not positional: libtest gives a
    /// test function no arguments.
    const BARE_FILENAME_CHILD: &str = "PMACS_TEST_BARE_FILENAME_CHILD";

    /// `Path::parent()` of a bare filename returns `Some("")` (empty path),
    /// not `None`. Treat that as "save in cwd".
    ///
    /// **The cwd is a child's, never this process's.** The obvious spelling
    /// — `set_current_dir` onto a `TempDir` and back — is a process-global
    /// mutation, and libtest runs this binary's tests on a thread pool, so
    /// it repoints the cwd of every *other* test running at that instant.
    /// `packages::fetcher`'s `run_git` spawns `git` with no explicit cwd
    /// (`run_git_inner`'s `None` arm), so those children inherit whatever
    /// this test had installed; the parent's restore does nothing for a
    /// child already running, and the `TempDir` then drops underneath a
    /// live `git`, which reports `Unable to read current working
    /// directory`. That is registry row U16, demonstrated by its own
    /// retirement experiment on 2026-09-08: 400 iterations of the fetcher
    /// module with this test in the process failed 3 times, 400 without it
    /// failed 0. A serial guard does not close it — the `git` child
    /// outlives any guard the parent can hold.
    ///
    /// So the cwd move happens in a subprocess: `Command::current_dir` is
    /// per-child and touches nothing here. The child is this same test
    /// binary re-executed against the one `#[ignore]`d body below, which
    /// keeps the subject `save_atomic` on a real bare path rather than a
    /// stand-in for it.
    #[test]
    fn bare_filename_saves_in_cwd() {
        let dir = TempDir::new().unwrap();
        let exe = std::env::current_exe().expect("this test binary");
        let out = std::process::Command::new(&exe)
            .args([
                "--exact",
                "--ignored",
                "--nocapture",
                "file_io::tests::bare_filename_saves_in_the_child_cwd",
            ])
            .current_dir(dir.path())
            .env(BARE_FILENAME_CHILD, "1")
            .output()
            .expect("re-execute this test binary as the child");
        assert!(
            out.status.success(),
            "the child saving a bare filename must succeed; status {:?}\n\
             stdout:\n{}\nstderr:\n{}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );

        // Verify from HERE that the file landed in the child's cwd: the
        // child asserting it could pass against a file it wrote anywhere.
        let (bytes, _) = load_file(&dir.path().join("bare.txt")).unwrap();
        assert_eq!(bytes, b"x");
    }

    /// The child half of [`bare_filename_saves_in_cwd`]: saves a bare
    /// filename into whatever cwd the parent gave this process.
    ///
    /// `#[ignore]` so the sweep never runs it on its own, where the cwd
    /// would be the workspace root and the save would litter it. The env
    /// var makes that refusal explicit rather than leaving a hand-typed
    /// `--ignored` run to write `bare.txt` into the repository.
    #[test]
    #[ignore = "child process of bare_filename_saves_in_cwd; needs that parent's cwd"]
    fn bare_filename_saves_in_the_child_cwd() {
        assert!(
            std::env::var_os(BARE_FILENAME_CHILD).is_some(),
            "run this only through bare_filename_saves_in_cwd, which sets \
             {BARE_FILENAME_CHILD} and a cwd to write into",
        );
        let meta =
            save_atomic(Path::new("bare.txt"), b"x").expect("bare filename should save in cwd");
        assert_eq!(meta.size, 1);
    }

    #[test]
    fn root_path_is_an_error() {
        // `Path::parent()` of "/" is None; that's the real "no parent" case.
        let result = save_atomic(Path::new("/"), b"x");
        assert!(matches!(result, Err(SaveError::NoParent(_))));
    }
}
