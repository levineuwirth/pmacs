// state.rs --- persistent editor state directory (Arc 3, Q#PS2).

//! The `$XDG_STATE_HOME/pmacs/` base that all persisted editor state
//! lives under: minibuffer history (the original tenant), plus the
//! Arc 3 persistence features (recent files, saveplace, desktop, and
//! autosave recovery).
//!
//! Env is passed in as arguments, never read inline, so the pure
//! resolver is testable without touching the process environment
//! (`#![forbid(unsafe_code)]` rules out `env::set_var`).

use std::ffi::OsStr;
use std::path::PathBuf;

/// The base state directory `.../pmacs`, or `None` when neither
/// `XDG_STATE_HOME` nor `HOME` is usably set.
///
/// Order: `$XDG_STATE_HOME/pmacs`, then `$HOME/.local/state/pmacs`.
///
/// A **blank** `XDG_STATE_HOME` is treated as *absent* (Q#PS2 fix): the
/// prior history resolver returned a *relative* `pmacs/…` for
/// `Some("")`, which would write state into the process's current
/// directory — a latent bug. Here an empty (or all-whitespace) value
/// falls through to `HOME`, and a `HOME` that is itself blank yields
/// `None` rather than a relative path.
#[must_use]
pub fn state_dir(xdg_state: Option<&OsStr>, home: Option<&OsStr>) -> Option<PathBuf> {
    // `XDG_STATE_HOME` must be an absolute path per the XDG spec; a
    // relative value would root state at a *cwd-relative* `pmacs/...`
    // (the same latent bug the empty case had). Ignore relative values
    // and fall through to `HOME`, which likewise must be absolute.
    if let Some(xdg) = xdg_state.filter(|s| !is_blank(s)) {
        let p = PathBuf::from(xdg);
        if p.is_absolute() {
            return Some(p.join("pmacs"));
        }
    }
    home.filter(|s| !is_blank(s)).and_then(|h| {
        let p = PathBuf::from(h);
        p.is_absolute()
            .then(|| p.join(".local").join("state").join("pmacs"))
    })
}

/// Resolve the base state directory from the process environment.
///
/// A `PMACS_STATE_HOME` override wins over `XDG_STATE_HOME`/`HOME` when
/// set (and non-blank): `.../pmacs` under it. This is the redirect a
/// test harness, CI, or a privacy-conscious user points at a scratch
/// dir so persistence never touches the real `~/.local/state/pmacs`
/// (integration tests link the lib without `cfg(test)`, so the
/// startup wiring runs — the override is how they stay clean).
#[must_use]
pub fn user_state_dir() -> Option<PathBuf> {
    if let Some(over) = std::env::var_os("PMACS_STATE_HOME")
        .as_deref()
        .filter(|s| !is_blank(s))
    {
        // The override must also be absolute — a relative redirect would
        // reintroduce the cwd-relative-state footgun.
        let p = PathBuf::from(over);
        if p.is_absolute() {
            return Some(p.join("pmacs"));
        }
    }
    state_dir(
        std::env::var_os("XDG_STATE_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )
}

/// True when `s` is empty or all ASCII/Unicode whitespace — an
/// unusable env value we treat as unset.
fn is_blank(s: &OsStr) -> bool {
    match s.to_str() {
        Some(text) => text.trim().is_empty(),
        // Non-UTF-8 path bytes are a real (if exotic) directory name;
        // only the empty OsStr counts as blank there.
        None => s.is_empty(),
    }
}

// ---------------------------------------------------------------------------
// Confined key→file store (Q#PS2)
// ---------------------------------------------------------------------------

use std::path::Path;

/// Validate a state key so `pmacs.state.*` can never escape the state
/// directory (Q#PS2 path confinement). A key is a **relative** path of
/// one or more `/`-separated components, each non-empty and drawn from
/// `[A-Za-z0-9._-]`, and no component may be `.` or `..`. Everything
/// else — an absolute path, an empty key, a `.`/`..` component, `//`,
/// or any other byte (separators, control chars, spaces) — is rejected.
///
/// Without this, a state binding meant to *avoid* raw `io.open` would
/// become an arbitrary read/write anywhere on disk.
///
/// # Errors
/// Returns a static message describing the first rule the key violates.
pub fn validate_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() {
        return Err("state key is empty");
    }
    // Reject a leading `/` up front so the split below can't be fooled.
    if name.starts_with('/') {
        return Err("state key must be relative");
    }
    let mut components = 0usize;
    for part in name.split('/') {
        if part.is_empty() {
            return Err("state key has an empty path component");
        }
        if part == "." || part == ".." {
            return Err("state key may not contain `.` or `..`");
        }
        if !part
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        {
            return Err("state key component has a disallowed character");
        }
        components += 1;
    }
    if components == 0 {
        return Err("state key is empty");
    }
    Ok(())
}

/// Resolve a validated key to its path under `base`, refusing any
/// route that could escape the state directory.
///
/// Two guards beyond [`validate_name`]'s lexical rules:
/// 1. a `starts_with(base)` belt (redundant with `validate_name`, kept
///    as defense in depth);
/// 2. **symlink confinement** — every *existing* component the key adds
///    under `base` is `lstat`'d, and a symlink (live *or* broken) is
///    rejected. Without this, a `base/autosave` symlink pointing at
///    `/tmp/out` would let `state.write("autosave/x", …)` write outside
///    `base` — the lexical check alone can't catch it. `base` itself may
///    be a symlink (a dotfile-managed `~/.local/state`); only the
///    components the *key* contributes are guarded.
///
/// # Errors
/// Propagates [`validate_name`], or errors on an escaping / symlinked key.
pub fn resolve(base: &Path, name: &str) -> Result<PathBuf, &'static str> {
    validate_name(name)?;
    let path = base.join(name);
    if !path.starts_with(base) {
        return Err("state key escapes the state directory");
    }
    let mut cur = base.to_path_buf();
    for part in name.split('/') {
        cur.push(part);
        if let Ok(meta) = std::fs::symlink_metadata(&cur)
            && meta.file_type().is_symlink()
        {
            return Err("state key traverses a symlink");
        }
    }
    Ok(path)
}

/// Read a state file's contents, or `Ok(None)` when it does not exist.
///
/// # Errors
/// Invalid key, or an IO error other than not-found.
pub fn read(base: &Path, name: &str) -> Result<Option<String>, StateError> {
    let path = resolve(base, name).map_err(StateError::Name)?;
    match std::fs::read_to_string(&path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(StateError::Io(e)),
    }
}

/// Atomically write `content` to a state file (creating parents),
/// via [`crate::file_io::save_atomic`] — same durability the editor's
/// own saves get, and no raw `io.open`.
///
/// # Errors
/// Invalid key, or a save failure.
pub fn write(base: &Path, name: &str, content: &[u8]) -> Result<(), StateError> {
    write_inner(base, name, content, None)
}

/// Like [`write()`], but the parent directory is created `0700` and the
/// file written `0600` (Arc 3 Q#AS11).
///
/// Autosave stores **unsaved file contents**, a different class of secret
/// from saveplace's cursor offsets or recentf's path list. The default
/// path would give a new recovery file the umask default (typically
/// `0644`) and its directory `0755` — leaving a recovery copy of an
/// unsaved edit to a `0600` file *more exposed than the original*. The
/// mode is applied to the temp before the rename, so there is no window
/// at a laxer mode.
///
/// Permissions are Unix-only; elsewhere this is [`write()`].
///
/// # Errors
/// Invalid key, or a save failure.
pub fn write_private(base: &Path, name: &str, content: &[u8]) -> Result<(), StateError> {
    write_inner(base, name, content, Some(0o600))
}

/// True when a state file exists (no read, no parse).
///
/// # Errors
/// Invalid key.
pub fn exists(base: &Path, name: &str) -> Result<bool, StateError> {
    let path = resolve(base, name).map_err(StateError::Name)?;
    Ok(path.exists())
}

fn write_inner(
    base: &Path,
    name: &str,
    content: &[u8],
    mode: Option<u32>,
) -> Result<(), StateError> {
    let path = resolve(base, name).map_err(StateError::Name)?;
    if let Some(parent) = path.parent() {
        create_dir_all_with_mode(parent, mode.map(|_| 0o700)).map_err(StateError::Io)?;
        // A directory *we* own beneath the state root (e.g. `autosave/`)
        // must actually be `0700`, even if a previous run — or a user —
        // created it laxer. Otherwise the mode only applies to the dirs
        // this call happened to create, and a pre-existing `0755`
        // `autosave/` would still leak recovery-file names, sizes, and
        // mtimes despite the `0600` contents.
        //
        // Never re-mode `base` itself: the state root is a directory the
        // user may already have, shared with history/recentf/desktop.
        if mode.is_some() && parent != base {
            enforce_dir_mode(parent, 0o700).map_err(StateError::Io)?;
        }
    }
    crate::file_io::save_atomic_with_mode(&path, content, mode).map_err(StateError::Save)?;
    Ok(())
}

/// `create_dir_all`, birthing any directory this call creates at `mode`
/// (so it is never briefly world-readable).
fn create_dir_all_with_mode(dir: &Path, mode: Option<u32>) -> std::io::Result<()> {
    #[cfg(unix)]
    if let Some(m) = mode {
        use std::os::unix::fs::DirBuilderExt as _;
        return std::fs::DirBuilder::new()
            .recursive(true)
            .mode(m)
            .create(dir);
    }
    #[cfg(not(unix))]
    let _ = mode;
    std::fs::create_dir_all(dir)
}

/// Tighten an existing directory to `mode` if it is laxer. No-op on
/// non-Unix, and cheap when already correct.
fn enforce_dir_mode(dir: &Path, mode: u32) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let current = std::fs::metadata(dir)?.permissions().mode() & 0o777;
        if current != mode {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode))?;
        }
    }
    #[cfg(not(unix))]
    let _ = (dir, mode);
    Ok(())
}

/// Read a state file's raw bytes, or `Ok(None)` when it does not exist.
///
/// [`read`] returns a `String` (`read_to_string`), which fails on
/// non-UTF-8 content. pmacs buffers hold arbitrary bytes, so an autosave
/// recovery file cannot be read that way (Arc 3 Q#AS4).
///
/// # Errors
/// Invalid key, or an IO error other than not-found.
pub fn read_bytes(base: &Path, name: &str) -> Result<Option<Vec<u8>>, StateError> {
    let path = resolve(base, name).map_err(StateError::Name)?;
    match std::fs::read(&path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(StateError::Io(e)),
    }
}

/// Remove a state file. Missing file is success (idempotent).
///
/// # Errors
/// Invalid key, or an IO error other than not-found.
pub fn remove(base: &Path, name: &str) -> Result<(), StateError> {
    let path = resolve(base, name).map_err(StateError::Name)?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(StateError::Io(e)),
    }
}

/// Error from a confined state operation.
#[derive(Debug)]
pub enum StateError {
    /// The key failed [`validate_name`].
    Name(&'static str),
    /// An underlying IO failure (read / remove).
    Io(std::io::Error),
    /// An atomic-write failure.
    Save(crate::file_io::SaveError),
}

impl std::fmt::Display for StateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StateError::Name(m) => write!(f, "invalid state key: {m}"),
            StateError::Io(e) => write!(f, "{e}"),
            StateError::Save(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for StateError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_xdg_state_home() {
        let d = state_dir(Some(OsStr::new("/x/state")), Some(OsStr::new("/home/u"))).unwrap();
        assert_eq!(d, PathBuf::from("/x/state/pmacs"));
    }

    #[test]
    fn falls_back_to_home_local_state() {
        let d = state_dir(None, Some(OsStr::new("/home/u"))).unwrap();
        assert_eq!(d, PathBuf::from("/home/u/.local/state/pmacs"));
    }

    #[test]
    fn none_when_neither_is_set() {
        assert!(state_dir(None, None).is_none());
    }

    #[test]
    fn validate_name_accepts_keys_and_subpaths() {
        for ok in ["recentf", "places", "autosave/deadbeef", "a.b_c-1/x2"] {
            assert!(validate_name(ok).is_ok(), "{ok:?} should be accepted");
        }
    }

    #[test]
    fn validate_name_rejects_escapes() {
        for bad in [
            "",
            "/etc/passwd",
            "..",
            "../x",
            "a/../b",
            "a//b",
            "a/",
            "/a",
            ".",
            "a/.",
            "with space",
            "tab\t",
            "null\0",
            "sub/../../x",
            "..\\x",
        ] {
            assert!(validate_name(bad).is_err(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn resolve_stays_under_base() {
        let base = PathBuf::from("/state/pmacs");
        assert_eq!(
            resolve(&base, "autosave/x").unwrap(),
            PathBuf::from("/state/pmacs/autosave/x")
        );
        assert!(resolve(&base, "../escape").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn resolve_rejects_symlink_components() {
        let root = std::env::temp_dir().join(format!("pmacs-symlink-{}", std::process::id()));
        let base = root.join("pmacs");
        let outside = root.join("outside");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        // A live symlink `base/evil -> outside` must be refused (else a
        // write through it escapes the state dir).
        let evil = base.join("evil");
        std::os::unix::fs::symlink(&outside, &evil).unwrap();
        assert!(resolve(&base, "evil/x").is_err(), "live symlink escape");
        assert!(write(&base, "evil/x", b"nope").is_err());
        assert!(!outside.join("x").exists(), "nothing was written outside");

        // A broken symlink component is also refused (lstat sees it).
        let broken = base.join("broken");
        std::os::unix::fs::symlink(root.join("does-not-exist"), &broken).unwrap();
        assert!(resolve(&base, "broken/y").is_err(), "broken symlink escape");

        // A plain subdir is fine.
        assert!(resolve(&base, "autosave/ok").is_ok());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn relative_xdg_and_home_are_ignored() {
        // A relative XDG_STATE_HOME (spec violation) must not root state
        // at a cwd-relative path; it falls through to HOME.
        let d = state_dir(Some(OsStr::new("relstate")), Some(OsStr::new("/home/u"))).unwrap();
        assert_eq!(d, PathBuf::from("/home/u/.local/state/pmacs"));
        // Relative XDG and relative HOME → None, never a relative root.
        assert!(state_dir(Some(OsStr::new("rel")), Some(OsStr::new("relhome"))).is_none());
    }

    #[test]
    fn write_read_remove_round_trip() {
        let dir = std::env::temp_dir().join(format!("pmacs-state-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(read(&dir, "recentf").unwrap().is_none(), "absent → None");
        write(&dir, "recentf", b"a\nb\n").unwrap();
        assert_eq!(read(&dir, "recentf").unwrap().as_deref(), Some("a\nb\n"));
        // Subpath creates its parent dir.
        write(&dir, "autosave/h1", b"x").unwrap();
        assert_eq!(read(&dir, "autosave/h1").unwrap().as_deref(), Some("x"));
        remove(&dir, "recentf").unwrap();
        assert!(read(&dir, "recentf").unwrap().is_none(), "removed → None");
        remove(&dir, "recentf").unwrap(); // idempotent
        // An invalid key errors rather than escaping.
        assert!(read(&dir, "../x").is_err());
        assert!(write(&dir, "/abs", b"x").is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_bytes_round_trips_non_utf8() {
        let dir = std::env::temp_dir().join(format!("pmacs-bytes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Invalid UTF-8 — what `read` (read_to_string) would choke on.
        let raw = [0xffu8, 0xfe, b'\n', 0x00, b'a'];
        write(&dir, "blob", &raw).unwrap();
        assert_eq!(read_bytes(&dir, "blob").unwrap().as_deref(), Some(&raw[..]));
        assert!(read(&dir, "blob").is_err(), "read_to_string rejects it");
        assert!(read_bytes(&dir, "absent").unwrap().is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn write_private_uses_0700_dir_and_0600_file() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("pmacs-priv-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();

        write_private(&dir, "autosave/secret", b"unsaved contents").unwrap();

        let file = dir.join("autosave").join("secret");
        let fmode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        assert_eq!(fmode, 0o600, "recovery file is 0600, not umask default");
        let dmode = std::fs::metadata(dir.join("autosave"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dmode, 0o700, "autosave dir is 0700");

        // Rewriting keeps the private mode (save_atomic inherits it).
        write_private(&dir, "autosave/secret", b"more").unwrap();
        let fmode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        assert_eq!(fmode, 0o600);

        // The plain `write` path is unchanged (umask default, not 0600).
        write(&dir, "plain", b"x").unwrap();
        let pmode = std::fs::metadata(dir.join("plain"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_ne!(pmode, 0o600, "plain write keeps existing behavior");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn blank_xdg_falls_through_to_home_not_a_relative_path() {
        // The Q#PS2 fix: an empty / whitespace XDG_STATE_HOME must NOT
        // produce a relative `pmacs/...` (which would write into the
        // cwd). It falls through to HOME instead.
        for blank in ["", "   ", "\t"] {
            let d = state_dir(Some(OsStr::new(blank)), Some(OsStr::new("/home/u"))).unwrap();
            assert_eq!(
                d,
                PathBuf::from("/home/u/.local/state/pmacs"),
                "blank XDG {blank:?} must fall through to HOME"
            );
            assert!(d.is_absolute(), "state dir is never relative");
        }
        // Blank XDG and no HOME → None, not a relative path.
        assert!(state_dir(Some(OsStr::new("")), None).is_none());
        // A blank HOME is likewise unusable.
        assert!(state_dir(None, Some(OsStr::new("  "))).is_none());
    }
}
