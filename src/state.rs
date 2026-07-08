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
    if let Some(xdg) = xdg_state.filter(|s| !is_blank(s)) {
        return Some(PathBuf::from(xdg).join("pmacs"));
    }
    home.filter(|s| !is_blank(s))
        .map(|h| PathBuf::from(h).join(".local").join("state").join("pmacs"))
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
        return Some(PathBuf::from(over).join("pmacs"));
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

/// Resolve a validated key to its absolute path under `base`, with a
/// canonical-prefix belt: the joined path must still start with `base`.
///
/// # Errors
/// Propagates [`validate_name`], or errors if the join escapes `base`
/// (which [`validate_name`] already prevents — this is defense in depth).
pub fn resolve(base: &Path, name: &str) -> Result<PathBuf, &'static str> {
    validate_name(name)?;
    let path = base.join(name);
    if !path.starts_with(base) {
        return Err("state key escapes the state directory");
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
    let path = resolve(base, name).map_err(StateError::Name)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(StateError::Io)?;
    }
    crate::file_io::save_atomic(&path, content).map_err(StateError::Save)?;
    Ok(())
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
